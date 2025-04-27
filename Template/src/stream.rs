use alloy::providers::{Provider, ProviderBuilder, Ipc};
use futures::StreamExt;
use log::{debug, warn};
use tokio::sync::broadcast::Sender;

use crate::events::Event;

/// Subscribes to new block headers over IPC and broadcasts them via a channel.
pub async fn stream_new_blocks(block_sender: Sender<Event>) {
    // 👇 Attempt to load IPC path from environment
    let ipc_path = std::env::var("IPC").expect("IPC path not set in environment");

    // 👇 Connect to the Ethereum node via IPC
    let ipc_conn = IpcConnect::new(ipc_path);
    let ipc = match ProviderBuilder::new().on_ipc(ipc_conn).await {
        Ok(p) => p,
        Err(e) => {
            warn!("Failed to connect via IPC: {:?}", e);
            return;
        }
    };

    // 👇 Subscribe to new block headers
    let sub = match ipc.subscribe_blocks().await {
        Ok(s) => s,
        Err(e) => {
            warn!("Failed to subscribe to new blocks: {:?}", e);
            return;
        }
    };

    let mut stream = sub.into_stream();

    // 👇 Stream and broadcast each new block as an Event
    while let Some(block) = stream.next().await {
        match block_sender.send(Event::NewBlock(block)) {
            Ok(_) => debug!("New block event sent"),
            Err(e) => warn!("Failed to broadcast new block: {:?}", e),
        }
    }
}
