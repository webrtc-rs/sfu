//! Payload-type translation through the `chat` SFU. A dedicated *sendonly* publisher registers
//! VP8 / Opus at *non-default* payload types (see [`common::custom_media_engine`]) and
//! publishes; *recvonly* subscribers register the browser defaults. A subscriber only ever
//! negotiates the payload types the SFU offers (the defaults), so the SFU must rewrite each
//! forwarded packet's payload type from the publisher's inbound value to the subscriber's
//! negotiated one — the translation exercised in `Room::poll_read`. Each subscriber asserts the
//! forwarded packets carry its own negotiated payload type, not the publisher's.
//!
//! The topology is deliberately unidirectional. A WebRTC peer uses a single payload type per
//! codec across all of its m-lines, so a peer that first subscribes (and negotiates the default
//! payload type) would then also *send* at that default — hiding the translation. Keeping
//! publishers sendonly and subscribers recvonly pins the publisher's inbound payload type to its
//! custom value.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use rand::random;
use rtc::peer_connection::configuration::media_engine::{MIME_TYPE_OPUS, MIME_TYPE_VP8};
use rtc::rtp_transceiver::RTCRtpTransceiverDirection;
use rtc::rtp_transceiver::rtp_sender::RtpCodecKind;

use webrtc::peer_connection::RTCSdpType;

mod common;

use common::{CUSTOM_OPUS_PAYLOAD_TYPE, CUSTOM_VP8_PAYLOAD_TYPE, HOST, SIGNAL_PORT, SendTrack};

/// How many contiguous packets each subscriber verifies after the stream starts flowing.
const VERIFY_COUNT: usize = 50;

/// The payload types a default-codec subscriber negotiates for VP8 / Opus, per
/// `register_default_codecs`. Distinct from the custom values the publisher sends, so a
/// forwarded packet carrying these proves the SFU translated it.
const DEFAULT_VP8_PAYLOAD_TYPE: u8 = 96;
const DEFAULT_OPUS_PAYLOAD_TYPE: u8 = 111;

/// One sendonly publisher (endpoint 0) publishes a VP8 and an Opus track at non-default payload
/// types; `subscriber_count` recvonly subscribers (endpoints 1..=N) each receive both forwarded
/// tracks and assert they arrive carrying the payload type the *subscriber* negotiated (a
/// default) rather than the publisher's inbound (custom) one — i.e. the SFU translated it.
async fn test_payload_type_translation(subscriber_count: u64) -> anyhow::Result<()> {
    let room_id = random::<u64>();

    // Publisher registers VP8/Opus at custom payload types; subscribers use the defaults.
    let mut publisher = common::connect_custom(HOST, SIGNAL_PORT, room_id, 0).await?;
    let mut subscribers = Vec::new();
    for i in 0..subscriber_count {
        subscribers.push(common::connect(HOST, SIGNAL_PORT, room_id, i + 1).await?);
    }

    // The publisher publishes a sendonly video and audio track.
    let video = Arc::new(
        publisher
            .add_track(
                MIME_TYPE_VP8,
                "video_track",
                RTCRtpTransceiverDirection::Sendonly,
            )
            .await?,
    );
    let audio = Arc::new(
        publisher
            .add_track(
                MIME_TYPE_OPUS,
                "audio_track",
                RTCRtpTransceiverDirection::Sendonly,
            )
            .await?,
    );
    // The publisher is the offerer and never subscribes, so its custom payload types win on the
    // publish leg — these are the SFU's inbound values.
    assert_eq!(video.payload_type, CUSTOM_VP8_PAYLOAD_TYPE);
    assert_eq!(audio.payload_type, CUSTOM_OPUS_PAYLOAD_TYPE);

    publisher.renegotiate().await?;
    // The SFU answers the publisher's offer and pushes each subscriber a re-offer.
    assert_eq!(RTCSdpType::Answer, publisher.next_sdp().await?.sdp_type);
    for subscriber in &mut subscribers {
        assert_eq!(RTCSdpType::Offer, subscriber.next_sdp().await?.sdp_type);
    }

    // Start publishing both tracks before awaiting the forwarded media: a subscriber's on_track
    // fires when the first forwarded RTP packet arrives, not merely on the subscribe SDP.
    let (video_writer, video_stop) = SendTrack::spawn_writer(video.clone());
    let (audio_writer, audio_stop) = SendTrack::spawn_writer(audio.clone());

    // Each subscriber receives the forwarded video and audio track and verifies the payload type.
    for subscriber in &mut subscribers {
        for _ in 0..2 {
            let remote = subscriber.next_track().await?;
            // The payload type this subscriber negotiated for the track's codec: the SFU's
            // default, which differs from the publisher's custom inbound payload type. Looked up
            // after `next_track` so the receiver is attached (see `negotiated_recv_payload_type`).
            let (mime, custom_pt, default_pt) = match remote.kind().await {
                RtpCodecKind::Audio => (
                    MIME_TYPE_OPUS,
                    CUSTOM_OPUS_PAYLOAD_TYPE,
                    DEFAULT_OPUS_PAYLOAD_TYPE,
                ),
                _ => (
                    MIME_TYPE_VP8,
                    CUSTOM_VP8_PAYLOAD_TYPE,
                    DEFAULT_VP8_PAYLOAD_TYPE,
                ),
            };
            let expected_pt = subscriber.negotiated_recv_payload_type(mime).await?;
            assert_eq!(expected_pt, default_pt);
            assert_ne!(expected_pt, custom_pt);
            common::verify_rtp_flow_payload_type(&remote, VERIFY_COUNT, expected_pt).await?;
        }
    }

    video_stop.store(true, Ordering::Relaxed);
    audio_stop.store(true, Ordering::Relaxed);
    let _ = video_writer.await;
    let _ = audio_writer.await;

    publisher.close().await?;
    for subscriber in subscribers {
        subscriber.close().await?;
    }
    Ok(())
}

#[tokio::test]
async fn test_payload_type_translation_2p() -> anyhow::Result<()> {
    test_payload_type_translation(1).await
}

#[tokio::test]
async fn test_payload_type_translation_3p() -> anyhow::Result<()> {
    test_payload_type_translation(2).await
}
