//! RTP forwarding through the `chat` SFU: peers publish VP8 tracks and assert the SFU
//! forwards them, intact and in order, to every other peer in the room.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use rand::random;
use rtc::peer_connection::configuration::media_engine::MIME_TYPE_VP8;
use rtc::rtp_transceiver::RTCRtpTransceiverDirection;

use webrtc::peer_connection::RTCSdpType;

mod common;

use common::{HOST, Peer, SIGNAL_PORT, SendTrack};

/// How many contiguous packets each subscriber verifies after the stream starts flowing.
const VERIFY_COUNT: usize = 50;

/// Publisher (endpoint 0, sendonly) → SFU → subscriber (endpoint 1). Verifies the publish
/// offer is answered, the SFU re-offers the subscriber, and RTP round-trips intact.
#[tokio::test]
async fn test_rtp_uni_direction_0sendonly_1recvonly() -> anyhow::Result<()> {
    let room_id = random::<u64>();
    let mut publisher = common::connect(HOST, SIGNAL_PORT, room_id, 0).await?;
    let mut subscriber = common::connect(HOST, SIGNAL_PORT, room_id, 1).await?;

    let send_track = Arc::new(
        publisher
            .add_track(
                MIME_TYPE_VP8,
                "video_track",
                RTCRtpTransceiverDirection::Sendonly,
            )
            .await?,
    );
    publisher.renegotiate().await?;

    // The SFU answers the publisher's offer and pushes the subscriber a re-offer.
    assert_eq!(RTCSdpType::Answer, publisher.next_sdp().await?.sdp_type);
    assert_eq!(RTCSdpType::Offer, subscriber.next_sdp().await?.sdp_type);

    // Start publishing before awaiting the forwarded track: the subscriber's on_track fires
    // when the first forwarded RTP packet arrives, not merely on the subscribe SDP.
    let (writer, stop) = send_track.spawn_writer();
    let remote = subscriber.next_track().await?;
    common::verify_rtp_flow(&remote, VERIFY_COUNT).await?;
    stop.store(true, Ordering::Relaxed);
    let _ = writer.await;

    publisher.close().await?;
    subscriber.close().await?;
    Ok(())
}

/// Every peer publishes a sendonly track; the SFU forwards it to all others. Verifies the
/// full N×(N−1) forwarding mesh delivers RTP intact and in order.
async fn test_rtp_bi_direction_sendrecv(endpoint_count: u64) -> anyhow::Result<()> {
    let room_id = random::<u64>();

    let mut peers = Vec::new();
    for client_id in 0..endpoint_count {
        peers.push(common::connect(HOST, SIGNAL_PORT, room_id, client_id).await?);
    }

    // Each peer publishes a track. Serialize publishes: wait for each publish answer before
    // the next peer offers, so the client (which has no glare/rollback) is never mid-offer
    // when the SFU pushes it a subscribe re-offer.
    let mut send_tracks = Vec::new();
    for peer in &mut peers {
        // A unique track id per publisher: forwarded tracks that share an msid would be
        // indistinguishable to a subscriber (only one on_track would fire).
        let track_id = format!("video_track_{}", peer.client_id);
        let send_track = Arc::new(
            peer.add_track(
                MIME_TYPE_VP8,
                &track_id,
                RTCRtpTransceiverDirection::Sendonly,
            )
            .await?,
        );
        peer.renegotiate().await?;
        wait_for_answer(peer).await?;
        send_tracks.push(send_track);
        // Let the subscribe re-offers this publish triggered on the other peers be
        // auto-answered before the next peer publishes, so no peer is mid-answer (and thus
        // in signaling glare, which this client has no rollback for) when it offers.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    // Start every publisher writing before any subscriber reads.
    let mut writers = Vec::new();
    let mut stops = Vec::new();
    for send_track in &send_tracks {
        let (writer, stop) = SendTrack::spawn_writer(send_track.clone());
        writers.push(writer);
        stops.push(stop);
    }

    // Each peer receives a forwarded track from every other peer; verify each flow.
    for peer in &mut peers {
        for _ in 0..(endpoint_count - 1) {
            let remote = peer.next_track().await?;
            common::verify_rtp_flow(&remote, VERIFY_COUNT).await?;
        }
    }

    for stop in &stops {
        stop.store(true, Ordering::Relaxed);
    }
    for writer in writers {
        let _ = writer.await;
    }
    for peer in peers {
        peer.close().await?;
    }
    Ok(())
}

/// Drain a peer's signaling until its own publish offer is answered, skipping (already
/// auto-answered) subscribe re-offers.
async fn wait_for_answer(peer: &mut Peer) -> anyhow::Result<()> {
    loop {
        if peer.next_sdp().await?.sdp_type == RTCSdpType::Answer {
            return Ok(());
        }
    }
}

#[tokio::test]
async fn test_rtp_2p_bi_direction_sendrecv() -> anyhow::Result<()> {
    test_rtp_bi_direction_sendrecv(2).await
}

#[tokio::test]
async fn test_rtp_3p_bi_direction_sendrecv() -> anyhow::Result<()> {
    test_rtp_bi_direction_sendrecv(3).await
}
