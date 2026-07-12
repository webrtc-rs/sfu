//! RTP header-extension-id translation through the `chat` SFU. A dedicated *sendonly* publisher
//! negotiates the transport-cc header extension at a *non-default* id (see
//! [`common::custom_ext_media_engine`]) and stamps it onto every packet; *recvonly* subscribers
//! use the browser defaults. A subscriber only ever negotiates the id the SFU offers (the
//! default), so the SFU must rewrite each forwarded packet's extension id from the publisher's
//! inbound value to the subscriber's negotiated one — the translation exercised in
//! `Room::poll_read`. Each subscriber asserts the forwarded packets carry its own negotiated
//! extension id (and the same extension payload), not the publisher's.
//!
//! Like the payload-type test, the topology is unidirectional: a WebRTC peer uses a single id per
//! extension across its m-lines, so a peer that first subscribes (negotiating the default id)
//! would then also *send* at that default. Keeping publishers sendonly and subscribers recvonly
//! pins the publisher's inbound extension id to its custom value.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use bytes::Bytes;
use rand::random;
use rtc::peer_connection::configuration::media_engine::{MIME_TYPE_OPUS, MIME_TYPE_VP8};
use rtc::rtp_transceiver::RTCRtpTransceiverDirection;
use rtc::sdp::extmap::TRANSPORT_CC_URI;

use webrtc::peer_connection::RTCSdpType;

mod common;

use common::{HOST, SIGNAL_PORT, SendTrack};

/// How many contiguous packets each subscriber verifies after the stream starts flowing.
const VERIFY_COUNT: usize = 50;

/// The transport-cc extension id a default subscriber negotiates: `register_default_interceptors`
/// registers MID(1), RID(2), RRID(3), then transport-cc(4). Distinct from the publisher's custom
/// id, so a forwarded packet carrying this id proves the SFU translated it.
const DEFAULT_TRANSPORT_CC_ID: u8 = 4;

/// The transport-cc extension id the custom publisher negotiates: `custom_ext_media_engine`
/// registers transport-cc first, so it lands at id 1. This is the SFU's inbound value.
const CUSTOM_TRANSPORT_CC_ID: u8 = 1;

/// The fixed extension payload every publisher stamps; subscribers assert it round-trips
/// byte-for-byte through the id translation.
const EXT_PAYLOAD: &[u8] = &[0xab, 0xcd];

/// One sendonly publisher (endpoint 0) publishes a VP8 and an Opus track, each stamped with the
/// transport-cc header extension at a non-default id; `subscriber_count` recvonly subscribers
/// (endpoints 1..=N) each receive both forwarded tracks and assert the extension arrives under
/// the id the *subscriber* negotiated (a default), with its payload intact — i.e. the SFU
/// translated the id.
async fn test_header_extension_id_translation(subscriber_count: u64) -> anyhow::Result<()> {
    let room_id = random::<u64>();

    // Publisher negotiates transport-cc at a custom id; subscribers use the default.
    let mut publisher = common::connect_ext_custom(HOST, SIGNAL_PORT, room_id, 0).await?;
    let mut subscribers = Vec::new();
    for i in 0..subscriber_count {
        subscribers.push(common::connect(HOST, SIGNAL_PORT, room_id, i + 1).await?);
    }

    // The publisher publishes a sendonly video and audio track, each stamping the transport-cc
    // extension (at the publisher's negotiated id) onto every packet.
    let video = Arc::new(
        publisher
            .add_track_with_extension(
                MIME_TYPE_VP8,
                "video_track",
                RTCRtpTransceiverDirection::Sendonly,
                TRANSPORT_CC_URI,
                Bytes::from_static(EXT_PAYLOAD),
            )
            .await?,
    );
    let audio = Arc::new(
        publisher
            .add_track_with_extension(
                MIME_TYPE_OPUS,
                "audio_track",
                RTCRtpTransceiverDirection::Sendonly,
                TRANSPORT_CC_URI,
                Bytes::from_static(EXT_PAYLOAD),
            )
            .await?,
    );
    // The publisher is the offerer and never subscribes, so its custom extension id wins on the
    // publish leg — this is the SFU's inbound value.
    let video_ext = video
        .extension
        .as_ref()
        .expect("publisher video track has a header extension");
    let audio_ext = audio
        .extension
        .as_ref()
        .expect("publisher audio track has a header extension");
    assert_eq!(video_ext.id, CUSTOM_TRANSPORT_CC_ID);
    assert_eq!(audio_ext.id, CUSTOM_TRANSPORT_CC_ID);

    publisher.renegotiate().await?;
    // The SFU answers the publisher's offer and pushes each subscriber a re-offer.
    assert_eq!(RTCSdpType::Answer, publisher.next_sdp().await?.sdp_type);
    for subscriber in &mut subscribers {
        assert_eq!(RTCSdpType::Offer, subscriber.next_sdp().await?.sdp_type);
    }

    // Start publishing both tracks before awaiting the forwarded media.
    let (video_writer, video_stop) = SendTrack::spawn_writer(video.clone());
    let (audio_writer, audio_stop) = SendTrack::spawn_writer(audio.clone());

    // Each subscriber receives the forwarded video and audio track and verifies the extension id.
    for subscriber in &mut subscribers {
        for _ in 0..2 {
            let remote = subscriber.next_track().await?;
            // The extension id this subscriber negotiated: the SFU's default, which differs from
            // the publisher's custom inbound id. Looked up after `next_track` so the receiver is
            // attached (see `negotiated_recv_extension_id`). Both m-lines share one negotiated id.
            let expected_id = subscriber
                .negotiated_recv_extension_id(TRANSPORT_CC_URI)
                .await?;
            assert_eq!(expected_id, DEFAULT_TRANSPORT_CC_ID);
            assert_ne!(expected_id, CUSTOM_TRANSPORT_CC_ID);
            common::verify_rtp_flow_extension(&remote, VERIFY_COUNT, expected_id, EXT_PAYLOAD)
                .await?;
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
async fn test_header_extension_id_translation_2p() -> anyhow::Result<()> {
    test_header_extension_id_translation(1).await
}

#[tokio::test]
async fn test_header_extension_id_translation_3p() -> anyhow::Result<()> {
    test_header_extension_id_translation(2).await
}
