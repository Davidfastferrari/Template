use crate::events::Event;
use crate::gas_station::GasStation;
use crate::gen::FlashSwap;
use alloy::hex;
use alloy::network::Ethereum;
use alloy::primitives::{Address, Bytes as AlloyBytes, FixedBytes};
use alloy::providers::{Provider, ProviderBuilder, RootProvider};
use alloy::alloy_network::TransactionBuilder;
use alloy::rpc::types::TransactionRequest;
use alloy_signer_wallet::LocalWallet;
use alloy_signer::local::PrivateKeySigner;
use alloy_signer::k256::SecretKey;
use alloy_sol_types::{SolCall};
use alloy_transports_http::{Http, Client as AlloyClient};
use tokio::sync::mpsc::{Sender, Receiver}; 
use log::info;
use reqwest::Client;
use serde_json::Value;
use std::str::FromStr;
use std::time::{Duration, Instant};
use std::{
    sync::{
        Arc,
    },
};

#[derive(Serialize, Deserialize, Debug)]
struct Point {
    x: i32,
    y: i32,
}

// Handles sending transactions
pub struct TransactionSender {
    wallet: EthereumWallet<PrivateKeySigner>,
    gas_station: Arc<GasStation>,
    contract_address: Address,
    client: Arc<Client>,
    provider: Arc<RootProvider<Http<HttpClient>>>,
    nonce: u64,
}

impl TransactionSender {
    pub async fn new(gas_station: Arc<GasStation>) -> Self {
        // construct a wallet
        let key = std::env::var("PRIVATE_KEY").expect("PRIVATE_KEY not set");
        let key_hex = hex::decode(&key).expect("Invalid hex");
        let key = SecretKey::from_bytes((&key_hex[..]).into()).expect("Invalid secret key");
        let signer = PrivateKeySigner::from(key);
        let wallet = EthereumWallet::from(signer);

        // Create persistent reqwest client
        let client = Client::builder()
            .pool_max_idle_per_host(10)
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .expect("Failed to create reqwest client");

        // Warm-up request
        let warmup_json = json!({
            "jsonrpc": "2.0",
            "method": "eth_blockNumber",
            "params": [],
            "id": 1
        });
        let _ = client
            .post("https://mainnet-sequencer.base.org")
            .json(&warmup_json)
            .send()
            .await
            .unwrap();

        // setup provider
        let http_provider = std::env::var("FULL").unwrap().parse::<Http<HttpClient>>().unwrap();
        let provider = Arc::new(ProviderBuilder::new().on_http(http_provider));

        let nonce = provider
            .get_transaction_count(std::env::var("ACCOUNT").unwrap().parse().unwrap())
            .await
            .unwrap();

        Self {
            wallet,
            gas_station,
            contract_address: std::env::var("SWAP_CONTRACT").unwrap().parse().unwrap(),
            client: Arc::new(client),
            provider,
            nonce,
        }
    }

    pub async fn send_transactions(&mut self, tx_receiver: Receiver<Event>) {
        while let Ok(Event::ValidPath((arb_path, profit, block_number))) = tx_receiver.recv() {
            info!("Sending path...");

            let converted_path: FlashSwap::SwapParams = arb_path.clone().into();
            let calldata = FlashSwap::executeArbitrageCall {
                arb: converted_path,
            }
            .abi_encode();

            let (max_fee, priority_fee) = self.gas_station.get_gas_fees(profit);

            let tx = TransactionBuilder::default()
                .with_to(self.contract_address)
                .with_nonce(self.nonce)
                .with_gas_limit(2_000_000)
                .with_chain_id(8453)
                .with_max_fee_per_gas(max_fee)
                .with_max_priority_fee_per_gas(priority_fee)
                .transaction_type(2)
                .with_input(AlloyBytes::from(calldata));
            self.nonce += 1;

            let tx_envelope = tx.build(&self.wallet).await.unwrap();
            let mut encoded_tx = vec![];
            tx_envelope.encode_2718(&mut encoded_tx);
            let rlp_hex = hex::encode_prefixed(encoded_tx);

            let tx_data = json!({
                "jsonrpc": "2.0",
                "method": "eth_sendRawTransaction",
                "params": [rlp_hex],
                "id": 1
            });

            info!("Sending on block {}", block_number);
            let start = Instant::now();

            let req = self
                .client
                .post("https://mainnet-sequencer.base.org")
                .json(&tx_data)
                .send()
                .await
                .unwrap();
            let req_response: Value = req.json().await.unwrap();
            info!("Took {:?} to send tx and receive response", start.elapsed());
            let tx_hash = FixedBytes::<32>::from_str(req_response["result"].as_str().unwrap())
                .unwrap();

            let provider = self.provider.clone();
            tokio::spawn(async move {
                Self::send_and_monitor(provider, tx_hash, block_number).await;
            });
        }
    }

    pub async fn send_and_monitor(
        provider: Arc<RootProvider<Http<HttpClient>>>,
        tx_hash: FixedBytes<32>,
        block_number: u64,
    ) {
        let mut attempts = 0;
        while attempts < 10 {
            let receipt = provider.get_transaction_receipt(tx_hash).await;
            if let Ok(Some(inner)) = receipt {
                info!(
                    "Sent on block {:?}, Landed on block {:?}",
                    block_number,
                    inner.block_number.unwrap()
                );
                return;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
            attempts += 1;
        }
    }
}

// Test transaction sending functionality
#[cfg(test)]
mod tx_signing_tests {
    use alloy::primitives::{address, U256};
    use alloy::providers::{Provider, ProviderBuilder};
    use env_logger;
    use gen::FlashQuoter;
    use pool_sync::PoolType;
    use std::time::Instant;
    use AMOUNT;
    use super::*;

    // Create mock swap params
    fn dummy_swap_params() ->  FlashQuoter::SwapParams {
        let p1 = address!("4C36388bE6F416A29C8d8Eee81C771cE6bE14B18");
        let p2 = address!("9A834b70C07C81a9FCB695573D9008d0eF23A998");
        FlashQuoter::SwapParams {
            pools: vec![p1, p2],
            poolVersions: vec![0, 0],
            amountIn: *AMOUNT
        }
    }


    // Test the time it takes to create a transaction
    #[tokio::test(flavor = "multi_thread")]
    async fn test_sign() {
        // init and get all dummy state
        dotenv::dotenv().ok();
        let key = std::env::var("PRIVATE_KEY").unwrap();
        let key_hex = hex::decode(key).unwrap();
        let key = SecretKey::from_bytes((&key_hex[..]).into()).unwrap();
        let signer = PrivateKeySigner::from(key);
        let wallet = EthereumWallet::from(signer);
        let url = std::env::var("FULL").unwrap();
        let wallet_provider = Arc::new(
            ProviderBuilder::new()
                .with_recommended_fillers()
                .wallet(wallet)
                .on_http(url),
        );
        let contract_address = std::env::var("SWAP_CONTRACT").unwrap();
        let contract = FlashSwap::new(contract_address.parse().unwrap(), wallet_provider.clone());
        let path: FlashSwap::SwapParams = dummy_swap_params().into();

        // benchmark tx construction
        let gas = wallet_provider.estimate_eip1559_fees(None).await.unwrap();
        let tx_time = Instant::now();
        let max_fee = gas.max_fee_per_gas * 5; // 3x the suggested max fee
        let priority_fee = gas.max_priority_fee_per_gas * 30; // 20x the suggested priority fee

        let _ = contract
            .executeArbitrage(path)
            .max_fee_per_gas(max_fee)
            .max_priority_fee_per_gas(priority_fee)
            .chain_id(8453)
            .gas(4_000_000)
            .into_transaction_request();
        println!("Tx construction took {:?}", tx_time.elapsed());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_send_tx() {
        // init environment
        env_logger::builder().filter_level(log::LevelFilter::Info);
        dotenv::dotenv().ok();

        // Create gas station
        let gas_station = Arc::new(GasStation::new());

        // Create transaction sender
        let mut tx_sender = TransactionSender::new(gas_station).await;

        // Create a channel for sending events
        let (tx, rx) = std::sync::mpsc::channel();

        // Create and send a test event
        let swap_path = dummy_swap_params();
        let test_event = Event::ValidPath((
            swap_path,
            alloy::primitives::U256::from(10000000), // test input amount
            100u64,                                  // dummy block number
        ));

        tx.send(test_event).unwrap();

        // Send the transaction (this will only process one transaction and then exit)
        tx_sender.send_transactions(rx).await;
    }
}

