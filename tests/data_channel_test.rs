//! Data-channel connectivity against the `chat` SFU: a peer that registers, publishes its
//! bootstrap offer, connects, and opens its data channel.

use rand::random;

mod common;

use common::{HOST, SIGNAL_PORT};

/// A single peer can register, connect, and open its data channel with the SFU.
#[tokio::test]
async fn test_data_channel() -> anyhow::Result<()> {
    let room_id = random::<u64>();
    let peer = common::connect(HOST, SIGNAL_PORT, room_id, 0).await?;
    peer.close().await?;
    Ok(())
}

/// Several peers can share one room, each establishing its own connection and data channel.
#[tokio::test]
async fn test_data_channels() -> anyhow::Result<()> {
    let room_id = random::<u64>();
    let mut peers = Vec::new();
    for client_id in 0..3u64 {
        peers.push(common::connect(HOST, SIGNAL_PORT, room_id, client_id).await?);
    }
    for peer in peers {
        peer.close().await?;
    }
    Ok(())
}
