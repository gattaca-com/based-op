use std::{
    collections::VecDeque,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_network::ReceiptResponse;
use alloy_primitives::{
    Address, B256, Bytes, U64, U256,
    map::foldhash::{HashMap, HashMapExt},
};
use alloy_provider::{Provider, ProviderBuilder, RootProvider, WsConnect, fillers::RecommendedFillers};
use alloy_rpc_types::Header;
use bop_common::{
    api::OpRpcBlock,
    p2p::{EnvV0, FragV0, SealV0, SignedVersionedMessage, StateUpdate, VersionedMessage},
    utils::{init_tracing, wait_for_signal},
};
use clap::Parser;
use cli::RpcArgs;
use crossbeam_channel::Sender;
use eyre::Result;
use futures_util::stream::StreamExt;
use http::Uri;
use jsonrpsee::{
    core::{RpcResult, async_trait}, http_client::HttpClientBuilder, server::{ServerBuilder, ServerConfigBuilder}, types::ErrorObject, ws_client::RpcServiceBuilder
};
use op_alloy_network::Optimism;
use op_alloy_rpc_types::OpTransactionReceipt;
use parking_lot::RwLock;
use reqwest::Url;
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};
use std::str::FromStr;
use tokio::time::interval;
use tokio_websockets::ClientBuilder;
use tracing::{debug, error, info, warn};

use crate::{middleware::{EthApiProxy, RpcClient}, types::EthApiServer};

mod cli;
mod types;
mod middleware;

type OpRootProvider = RootProvider<Optimism>;

struct UnsealedBlock {
    env: EnvV0,
    current_frag: Option<FragV0>,
    transaction_count_diff: HashMap<Address, u64>,
    receipts: HashMap<B256, OpTransactionReceipt>,
    balances: HashMap<Address, U256>,
    seal: Option<SealV0>,
}

impl UnsealedBlock {
    fn apply_frag(&mut self, frag: FragV0, state_update: Option<StateUpdate>) {
        if self.current_frag.is_none() {
            if frag.seq != 0 {
                error!("expected first frag to have seq 0 but got seq {}", frag.seq);
                return;
            }
        } else {
            let current_frag = self.current_frag.as_ref().unwrap();
            let expected_seq = current_frag.seq + 1;
            if expected_seq != frag.seq {
                error!("expected frag seq {} but got seq {}", expected_seq, frag.seq);
                return;
            }
        }
        if self.seal.is_some() {
            error!("trying to apply frag after seal");
            return;
        }

        self.current_frag = Some(frag);

        if let Some(state_update) = state_update {
            for (_tx_hash, receipt) in state_update.receipts.iter() {
                let sender = receipt.from();
                self.transaction_count_diff.entry(sender).and_modify(|count| *count += 1).or_insert(1);
            }
            self.receipts.extend(state_update.receipts);
            self.balances.extend(state_update.balances);
        }
    }

    fn apply_seal(&mut self, seal: SealV0) {
        self.seal = Some(seal);
    }

    fn get_transaction_count_diff(&self, address: Address) -> Option<u64> {
        self.transaction_count_diff.get(&address).cloned()
    }

    fn get_receipt(&self, tx_hash: B256) -> Option<OpTransactionReceipt> {
        self.receipts.get(&tx_hash).cloned()
    }

    fn get_balance(&self, address: Address) -> Option<U256> {
        self.balances.get(&address).cloned()
    }
}

struct UnsealedBlockStack {
    blocks: VecDeque<UnsealedBlock>,
    root_provider_block_number: Option<u64>,
}

impl UnsealedBlockStack {
    fn new() -> Self {
        Self { blocks: VecDeque::new(), root_provider_block_number: None }
    }

    fn get_transaction_count_diff(&self, address: Address) -> u64 {
        let mut total_diff = 0;
        for block in self.blocks.iter().rev() {
            total_diff += block.get_transaction_count_diff(address).unwrap_or(0);
        }
        total_diff
    }

    fn get_receipt(&self, tx_hash: B256) -> Option<OpTransactionReceipt> {
        for block in self.blocks.iter().rev() {
            if let Some(receipt) = block.get_receipt(tx_hash) {
                return Some(receipt);
            }
        }
        None
    }

    fn get_balance(&self, address: Address) -> Option<U256> {
        for block in self.blocks.iter().rev() {
            if let Some(balance) = block.get_balance(address) {
                return Some(balance);
            }
        }
        None
    }

    fn block_number(&self) -> Option<u64> {
        if let Some(block) = self.blocks.back() {
            return Some(block.env.number);
        }
        if let Some(root_provider_block_number) = self.root_provider_block_number {
            return Some(root_provider_block_number);
        }
        None
    }
}

pub fn spawn_receipt_listener_frag_stream(frag_url: &str, message_tx: Sender<SignedVersionedMessage>) {
    let frag_url = frag_url.to_string();
    tokio::spawn(async move {
        loop {
            let uri = Uri::from_str(&frag_url).expect("invalid frag stream url");
            let maybe_client = ClientBuilder::from_uri(uri).connect().await;
            let Ok((mut client, _)) = maybe_client else {
                error!("failed to connect to frag stream, reconnecting...");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            };
            while let Some(Ok(msg)) = client.next().await {
                let stream_data = msg.as_text().and_then(|s| serde_json::from_str::<SignedVersionedMessage>(s).ok());
                if let Some(msg) = stream_data {
                    message_tx.send(msg).expect("failed to send message");
                }
            }
            error!("frag stream closed, reconnecting...");
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

#[derive(Clone)]
struct Server {
    unsealed_stack: Arc<RwLock<UnsealedBlockStack>>,
    provider: OpRootProvider,
    tx_receiver_provider: OpRootProvider,
}

impl Server {
    fn new(provider: OpRootProvider, tx_receiver_provider: OpRootProvider) -> Self {
        Self { unsealed_stack: Arc::new(RwLock::new(UnsealedBlockStack::new())), provider, tx_receiver_provider }
    }

    fn on_env(&self, env: EnvV0) {
        let mut unsealed_block = self.unsealed_stack.upgradable_read();
        if unsealed_block.blocks.is_empty() || unsealed_block.blocks.back().unwrap().env.number + 1 == env.number {
            unsealed_block.with_upgraded(|blocks| {
                blocks.blocks.push_back(UnsealedBlock {
                    env,
                    current_frag: None,
                    transaction_count_diff: HashMap::new(),
                    receipts: HashMap::new(),
                    balances: HashMap::new(),
                    seal: None,
                });
            });
        } else {
            error!("expected block number");
        }
    }

    fn on_header(&self, header: Header) {
        let mut unsealed_block = self.unsealed_stack.upgradable_read();
        while !unsealed_block.blocks.is_empty() && unsealed_block.blocks.front().unwrap().env.number <= header.number {
            unsealed_block.with_upgraded(|blocks| {
                blocks.blocks.pop_front();
                blocks.root_provider_block_number = Some(header.number);
            });
        }
    }

    fn on_seal(&self, seal: SealV0) {
        let mut unsealed_block = self.unsealed_stack.upgradable_read();
        if unsealed_block.blocks.is_empty() {
            // error!("trying to seal a block but there is no unsealed block");
            return;
        }
        let last_block = unsealed_block.blocks.back().unwrap(); // unwrap is safe because we just checked that the stack is not empty
        if last_block.env.number != seal.block_number {
            error!(
                "trying to seal block number {} but the last unsealed block is number {}",
                seal.block_number, last_block.env.number
            );
            return;
        }
        unsealed_block.with_upgraded(|blocks| {
            blocks.blocks.back_mut().unwrap().apply_seal(seal);
        });
    }

    fn on_frag(&self, frag: FragV0, state_update: Option<StateUpdate>) {
        let mut unsealed_block = self.unsealed_stack.upgradable_read();
        if unsealed_block.blocks.is_empty() {
            return;
        }
        let last_block = unsealed_block.blocks.back().unwrap(); // unwrap is safe because we just checked that the stack is not empty
        if last_block.env.number != frag.block_number {
            error!(
                "trying to apply frag for block number {} but the last unsealed block is number {}",
                frag.block_number, last_block.env.number
            );
            return;
        }
        unsealed_block.with_upgraded(|blocks| {
            blocks.blocks.back_mut().unwrap().apply_frag(frag, state_update);
        });
    }

    async fn get_transaction_count(&self, address: Address) -> Result<u64> {
        let stack = self.unsealed_stack.read();
        let transaction_count_diff = stack.get_transaction_count_diff(address);
        let root_provider_block_number = stack.root_provider_block_number;
        if let Some(root_provider_block_number) = root_provider_block_number {
            let transaction_count = self
                .provider
                .get_transaction_count(address)
                .block_id(BlockId::number(root_provider_block_number))
                .await?;
            return Ok(transaction_count_diff + transaction_count);
        }
        let transaction_count = self.provider.get_transaction_count(address).await?;
        Ok(transaction_count_diff + transaction_count)
    }

    async fn get_balance(&self, address: Address) -> Result<U256> {
        let stack = self.unsealed_stack.read();
        let balance = stack.get_balance(address);
        if let Some(balance) = balance {
            return Ok(balance);
        }
        let balance = self.provider.get_balance(address).await?;
        Ok(balance)
    }

    async fn get_receipt(&self, hash: B256) -> Result<Option<OpTransactionReceipt>> {
        let stack = self.unsealed_stack.read();
        if let Some(receipt) = stack.get_receipt(hash) {
            return Ok(Some(receipt));
        }
        let receipt = self.tx_receiver_provider.get_transaction_receipt(hash).await?;
        Ok(receipt)
    }

    async fn block_number(&self) -> Result<u64> {
        let stack = self.unsealed_stack.read();
        if let Some(block_number) = stack.block_number() {
            return Ok(block_number);
        } else {
            return Ok(self.provider.get_block_number().await?);
        }
    }
}

pub fn spawn_block_listener(provider: OpRootProvider, block_tx: Sender<Header>) {
    tokio::spawn(async move {
        loop {
            info!("Attempting to subscribe to L1 block headers...");
            let sub_result = provider.subscribe_blocks().await;

            let mut block_stream = match sub_result {
                Ok(sub) => {
                    info!("Successfully subscribed to L1 block headers.");
                    sub.into_stream()
                }
                Err(e) => {
                    error!(error = %e, "Failed to subscribe to L1 blocks, retrying in 5s");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };
            while let Some(header) = block_stream.next().await {
                block_tx.send(header).expect("failed to send block header");
            }
            warn!("header stream ended. Attempting to resubscribe...");
            panic!("WS connection dropped. Restart the process."); // TODO: handle reconnection
            // properly
        }
    });
}

#[tokio::main(flavor = "multi_thread", worker_threads = 10)]
async fn main() -> eyre::Result<()> {
    let args = RpcArgs::parse();
    let _guard = init_tracing((&args).into());

    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), args.port);

    let (block_tx, block_rx) = crossbeam_channel::bounded(100);
    let (message_tx, message_rx) = crossbeam_channel::bounded(100);

    let eth_ws_url = args.eth_ws_url.clone();
    let provider_with_filler = ProviderBuilder::<_, _, Optimism>::default()
        .connect_ws(WsConnect::new(Url::parse(&eth_ws_url).unwrap()))
        .await
        .expect("failed to connect to eth rpc");

    let provider = provider_with_filler.root();

    spawn_receipt_listener_frag_stream(&args.frag_url.as_str(), message_tx);
    spawn_block_listener(provider.clone(), block_tx);

    let tx_receiver_provider = match args.tx_receiver_url {
        Some(url) => {
            let parsed_url = Url::parse(&url).expect("invalid tx receiver url");
            let provider_with_filler = match parsed_url.scheme() {
                "ws" | "wss" => ProviderBuilder::<_, _, Optimism>::default()
                    .connect_ws(WsConnect::new(parsed_url))
                    .await
                    .expect("failed to connect to tx receiver via ws"),
                "http" | "https" => ProviderBuilder::<_, _, Optimism>::default().connect_http(parsed_url),
                _ => panic!("unsupported URL scheme for tx receiver: {}", parsed_url.scheme()),
            };
            provider_with_filler.root().clone()
        }
        None => provider.clone(),
    };

    let server_obj = Server::new(provider.clone(), tx_receiver_provider);

    let server = server_obj.clone();
    thread::spawn(move || {
        loop {
            let mut should_sleep = true;

            while let Ok(msg) = message_rx.try_recv() {
                match msg.message {
                    VersionedMessage::FragV0(frag) => {
                        debug!("got frag: block number {} seq {}", frag.block_number, frag.seq);
                        server.on_frag(frag, msg.state_update);
                    }
                    VersionedMessage::SealV0(seal) => {
                        debug!("got seal: block number {}", seal.block_number);
                        server.on_seal(seal);
                        if msg.state_update.is_some() {
                            error!("seal message should not contain state update");
                        }
                    }
                    VersionedMessage::EnvV0(env) => {
                        debug!("got env: block number {}", env.number);
                        server.on_env(env);
                        if msg.state_update.is_some() {
                            error!("env message should not contain state update");
                        }
                    }
                    _ => {
                        warn!("unsupported message type: {:?}", msg.message);
                        if msg.state_update.is_some() {
                            error!("unsupported message type should not contain state update");
                        }
                    }
                }
                should_sleep = false;
            }

            while let Ok(header) = block_rx.try_recv() {
                debug!("got block header: block number {}", header.number);
                server.on_header(header);
                should_sleep = false;
            }

            if should_sleep {
                thread::sleep(Duration::from_millis(1));
            }
        }
    });

    let server = server_obj.clone();
    tokio::spawn(async move {
        let address_to_check = Address::from_str("0x4D36DE6a194dDF98EE57323CfA3A45351d35e442").unwrap();
        let mut interval = interval(Duration::from_secs_f64(0.1));
        loop {
            let transaction_count = server.get_transaction_count(address_to_check).await.unwrap();
            let balance = server.get_balance(address_to_check).await.unwrap();
            let block_number = server.block_number().await.unwrap();
            info!("block number: {} count: {} balance: {:?}", block_number, transaction_count, balance);
            interval.tick().await;
        }
    });

    // temp: remove when factoring out the portal
    let cors = CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any);
    let cors_middleware = ServiceBuilder::new().layer(cors);

    let rpc_middleware = RpcServiceBuilder::new().layer_fn(move |s| EthApiProxy {
        inner: s,
        geth_client: create_client(Url::parse(args.eth_http_url.as_str()).unwrap(), Duration::from_secs(2)).unwrap(),
    });

    let rpc_server = ServerBuilder::default()
        .set_config(ServerConfigBuilder::new().max_request_body_size(u32::MAX).max_response_body_size(u32::MAX).build())
        .set_rpc_middleware(rpc_middleware)
        .set_http_middleware(cors_middleware)
        .build(addr)
        .await?;

    let mut module = EthApiServer::into_rpc(server_obj);
    let server_handle = rpc_server.start(module);


    tokio::select! {
        _ = server_handle.stopped() => {
            error!("server stopped");
        }

        _ = wait_for_signal() => {
            info!("received signal, shutting down");
        }
    }

    Ok(())
}

#[async_trait]
impl EthApiServer for Server {
    async fn send_raw_transaction(&self, bytes: Bytes) -> RpcResult<B256> {
        let hash = self.tx_receiver_provider.send_raw_transaction(&bytes).await.map_err(|e| {
            ErrorObject::owned(
                jsonrpsee::types::error::INTERNAL_ERROR_CODE,
                "Failed to send transaction",
                Some(e.to_string()),
            )
        })?;
        Ok(*hash.tx_hash())
    }

    async fn transaction_receipt(&self, hash: B256) -> RpcResult<Option<OpTransactionReceipt>> {
        let receipt = self.get_receipt(hash).await.map_err(|e| {
            ErrorObject::owned(
                jsonrpsee::types::error::INTERNAL_ERROR_CODE,
                "Failed to get transaction receipt",
                Some(e.to_string()),
            )
        })?;
        Ok(receipt)
    }

    async fn block_by_number(&self, number: BlockNumberOrTag, full: bool) -> RpcResult<Option<OpRpcBlock>> {
        todo!()
    }

    async fn block_by_hash(&self, hash: B256, full: bool) -> RpcResult<Option<OpRpcBlock>> {
        todo!()
    }

    async fn block_number(&self) -> RpcResult<U256> {
        let block_number = self.block_number().await.map_err(|e| {
            ErrorObject::owned(
                jsonrpsee::types::error::INTERNAL_ERROR_CODE,
                "Failed to get block number",
                Some(e.to_string()),
            )
        })?;
        Ok(U256::from(block_number))
    }

    async fn transaction_count(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256> {
        match block_number {
            Some(BlockId::Number(BlockNumberOrTag::Pending)) => {
                let block_number = self.block_number().await.map_err(|e| {
                    ErrorObject::owned(
                        jsonrpsee::types::error::INTERNAL_ERROR_CODE,
                        "Failed to get block number",
                        Some(e.to_string()),
                    )
                })?;
                Ok(U256::from(block_number))
            }
            _ => {
                let transaction_count = self
                    .provider
                    .get_transaction_count(address)
                    .block_id(block_number.unwrap_or_default())
                    .await
                    .map_err(|e| {
                        ErrorObject::owned(
                            jsonrpsee::types::error::INTERNAL_ERROR_CODE,
                            "Failed to get transaction count",
                            Some(e.to_string()),
                        )
                    })?;
                Ok(U256::from(transaction_count))
            }
        }
    }

    async fn balance(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256> {
        match block_number {
            Some(BlockId::Number(BlockNumberOrTag::Pending)) => {
                let block_number = self.block_number().await.map_err(|e| {
                    ErrorObject::owned(
                        jsonrpsee::types::error::INTERNAL_ERROR_CODE,
                        "Failed to get block number",
                        Some(e.to_string()),
                    )
                })?;
                Ok(U256::from(block_number))
            }
            _ => {
                let balance =
                    self.provider.get_balance(address).block_id(block_number.unwrap_or_default()).await.map_err(
                        |e| {
                            ErrorObject::owned(
                                jsonrpsee::types::error::INTERNAL_ERROR_CODE,
                                "Failed to get balance",
                                Some(e.to_string()),
                            )
                        },
                    )?;
                Ok(U256::from(balance))
            }
        }
    }
}

pub fn create_client(url: Url, timeout: Duration) -> eyre::Result<RpcClient> {
    let client = HttpClientBuilder::default()
        .max_request_size(u32::MAX)
        .max_response_size(u32::MAX)
        .request_timeout(timeout.into())
        .build(url)?;
    Ok(client)
}