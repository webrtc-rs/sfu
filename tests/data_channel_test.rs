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

/// Multiple concurrent rooms can be active at the same time on the same signaling/media server.
#[tokio::test]
async fn test_multiple_concurrent_rooms() -> anyhow::Result<()> {
    let mut room_handles = Vec::new();

    // Spawn 5 rooms concurrently, each room having 2 clients.
    for _ in 0..5 {
        let room_id = random::<u64>();
        let handle = tokio::spawn(async move {
            let peer1 = common::connect(HOST, SIGNAL_PORT, room_id, 0).await?;
            let peer2 = common::connect(HOST, SIGNAL_PORT, room_id, 1).await?;

            // Let them stay connected for a moment
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;

            peer1.close().await?;
            peer2.close().await?;
            anyhow::Result::<()>::Ok(())
        });
        room_handles.push(handle);
    }

    for handle in room_handles {
        handle.await??;
    }
    Ok(())
}
