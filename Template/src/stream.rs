use crate::events::Event;
use alloy::providers::{ IpcConnect, Provider, ProviderBuilder };
use futures::StreamExt;
use log::{ debug, warn };
use tokio::sync::broadcast::Sender;

// Stream in new blocks
pub async fn stream_new_blocks(block_sender: Sender<Event>) {
    // Construct ipc provider
    let ipc_conn = IpcConnect::new(std::env::var("IPC").unwrap());
    let ipc = ProviderBuilder::new().on_ipc(ipc_conn).await.unwrap();

    // Subscribe to new block stream
    let sub = ipc.subscribe_blocks().await.unwrap();
    let mut stream = sub.into_stream();

    while let Some(block) = stream.next().await {
        match block_sender.send(Event::NewBlock(block)) {
            Ok(_) => debug!("Block sent"),
            Err(e) => warn!("Block send failed: {:?}", e),
        }
    }
}
