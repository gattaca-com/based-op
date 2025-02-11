use std::{net::SocketAddr, time::Duration};

use alloy_primitives::{Bytes, B256};
use alloy_rpc_types::engine::{ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, PayloadId, PayloadStatus};
use bop_common::{
    api::{EngineApiClient, EngineApiServer, EthApiClient, MinimalEthApiServer, CAPABILITIES},
    communication::messages::{RpcError, RpcResult},
    utils::{uuid, wait_for_signal},
};
use jsonrpsee::{
    core::async_trait,
    http_client::{transport::HttpBackend, HttpClientBuilder},
    server::{RpcServiceBuilder, ServerBuilder},
};
use op_alloy_rpc_types_engine::{OpExecutionPayloadEnvelopeV3, OpPayloadAttributes};
use reth_rpc_layer::{AuthClientLayer, AuthClientService};
use tracing::{debug, error, info, Instrument, Level};

use crate::{cli::PortalArgs, middleware::ProxyService};

pub type HttpClient = jsonrpsee::http_client::HttpClient<AuthClientService<HttpBackend>>;

#[derive(Clone)]
pub struct PortalServer {
    fallback_client: HttpClient,
    gateway_client: HttpClient,
}

impl PortalServer {
    pub fn new(args: PortalArgs) -> eyre::Result<Self> {
        let gateway_jwt = args.gateway_jwt()?;
        let fallback_jwt = args.fallback_jwt()?;

        let secret_layer = AuthClientLayer::new(gateway_jwt);
        let middleware = tower::ServiceBuilder::default().layer(secret_layer);

        let gateway_client = HttpClientBuilder::default()
            .set_http_middleware(middleware)
            .request_timeout(Duration::from_millis(args.gateway_timeout_ms))
            .build(args.gateway_url)?;

        let secret_layer = AuthClientLayer::new(fallback_jwt);
        let middleware = tower::ServiceBuilder::default().layer(secret_layer);

        let fallback_client = HttpClientBuilder::default()
            .set_http_middleware(middleware)
            .request_timeout(Duration::from_millis(args.fallback_timeout_ms))
            .build(args.fallback_url)?;

        Ok(Self { fallback_client, gateway_client })
    }

    pub async fn run(self, addr: SocketAddr) -> eyre::Result<()> {
        let fallback_client = self.fallback_client.clone();
        let rpc_middleware =
            RpcServiceBuilder::new().layer_fn(move |s| ProxyService::new(CAPABILITIES, s, fallback_client.clone()));

        let server = ServerBuilder::default().set_rpc_middleware(rpc_middleware).build(addr).await?;

        let mut module = EngineApiServer::into_rpc(self.clone());
        module.merge(MinimalEthApiServer::into_rpc(self)).expect("failed to merge modules");

        let server_handle = server.start(module);

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
}

/// This is a temporary API to broacast transactions to both gateway and fallback. In practice this should not be
/// receiving user facing calls so we need to find another way to do this
#[async_trait]
impl MinimalEthApiServer for PortalServer {
    #[tracing::instrument(skip_all, err, ret(level = Level::DEBUG), fields(req_id = %uuid()))]
    async fn send_raw_transaction(&self, bytes: Bytes) -> RpcResult<B256> {
        // send both to gateway and fallback
        let gateway_fut = tokio::spawn({
            let client = self.gateway_client.clone();
            let bytes = bytes.clone();
            async move { client.send_raw_transaction(bytes).await }
        });

        let fallback_fut = tokio::spawn({
            let client = self.fallback_client.clone();
            async move { client.send_raw_transaction(bytes).await }
        });

        let (gateway, fallback) = tokio::join!(gateway_fut, fallback_fut);

        let gateway = gateway?;
        let fallback = fallback?;

        let response = gateway.or(fallback)?;

        Ok(response)
    }
}

#[async_trait]
impl EngineApiServer for PortalServer {
    #[tracing::instrument(skip_all, err, ret(level = Level::DEBUG), fields(req_id = %uuid()))]
    async fn fork_choice_updated_v3(
        &self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<OpPayloadAttributes>,
    ) -> RpcResult<ForkchoiceUpdated> {
        let parent_block_hash = fork_choice_state.head_block_hash;

        if let Some(payload_attributes) = payload_attributes.as_ref() {
            let no_tx_pool = payload_attributes.no_tx_pool.unwrap_or(false);
            let gas_limit = payload_attributes.gas_limit.unwrap_or(0);
            debug!(parent_block_hash = %parent_block_hash, no_tx_pool = %no_tx_pool, gas_limit = %gas_limit, "new request (with attributes)");
        } else {
            debug!(%parent_block_hash, "new request (no attributes)");
        }

        tokio::spawn(
            {
                let client = self.gateway_client.clone();
                let payload_attributes = payload_attributes.clone();

                async move {
                    match client.fork_choice_updated_v3(fork_choice_state, payload_attributes).await {
                        Ok(res) => {
                            if res.is_valid() {
                                debug!(?res, "gateway response");
                            } else {
                                error!(?res, "gateway response");
                            }
                        }
                        Err(err) => error!(%err, "failed gateway"),
                    }
                }
            }
            .in_current_span(),
        );

        let response = self.fallback_client.fork_choice_updated_v3(fork_choice_state, payload_attributes).await?;

        Ok(response)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::DEBUG), fields(req_id = %uuid()))]
    async fn new_payload_v3(
        &self,
        payload: ExecutionPayloadV3,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
    ) -> RpcResult<PayloadStatus> {
        let block_number = payload.payload_inner.payload_inner.block_number;
        let block_hash = payload.payload_inner.payload_inner.block_hash;
        let gas_limit = payload.payload_inner.payload_inner.gas_limit;
        let gas_used = payload.payload_inner.payload_inner.gas_used;
        let n_txs = payload.payload_inner.payload_inner.transactions.len();
        let n_withdrawals = payload.payload_inner.withdrawals.len();
        let blob_gas_used = payload.blob_gas_used;
        let excess_blob_gas = payload.excess_blob_gas;

        debug!(block_number, %block_hash, gas_limit, gas_used, n_txs, n_withdrawals, blob_gas_used, excess_blob_gas, "new request");

        tokio::spawn(
            {
                let client = self.gateway_client.clone();
                let payload = payload.clone();
                let versioned_hashes = versioned_hashes.clone();

                async move {
                    match client.new_payload_v3(payload, versioned_hashes, parent_beacon_block_root).await {
                        Ok(res) => {
                            if res.is_valid() {
                                debug!(?res, "gateway response");
                            } else {
                                error!(?res, "gateway response");
                            }
                        }
                        Err(err) => error!(?err, "failed gateway"),
                    }
                }
            }
            .in_current_span(),
        );

        let response = self.fallback_client.new_payload_v3(payload, versioned_hashes, parent_beacon_block_root).await?;

        Ok(response)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::DEBUG), fields(req_id = %uuid()))]
    async fn get_payload_v3(&self, payload_id: PayloadId) -> RpcResult<OpExecutionPayloadEnvelopeV3> {
        debug!(%payload_id, "new request");

        let fallback_fut = tokio::spawn({
            let client = self.fallback_client.clone();

            async move { client.get_payload_v3(payload_id).await }
        });

        let gateway_fut: tokio::task::JoinHandle<Result<OpExecutionPayloadEnvelopeV3, _>> = tokio::spawn(
            {
                let gateway_client = self.gateway_client.clone();
                let fallback_client = self.fallback_client.clone();

                async move {
                    let gateway_payload = gateway_client
                        .get_payload_v3(payload_id)
                        .await
                        .inspect_err(|err| error!(%err, "failed gateway"))?;
                    let payload_status = fallback_client
                        .new_payload_v3(
                            gateway_payload.execution_payload.clone(),
                            vec![],
                            gateway_payload.parent_beacon_block_root,
                        )
                        .await
                        .inspect_err(|err| error!(%err, "failed fallback validation"))?;

                    if payload_status.is_valid() {
                        debug!(?gateway_payload, ?payload_status, "gateway response");
                        Ok(gateway_payload)
                    } else {
                        error!(?gateway_payload, ?payload_status, "gateway response");
                        Err(RpcError::Internal)
                    }
                }
            }
            .in_current_span(),
        );

        let (fallback, gateway) = tokio::join!(fallback_fut, gateway_fut);

        // ignore join errors
        let fallback = fallback?;
        let gateway = gateway?;

        let payload = gateway.or(fallback)?;

        Ok(payload)
    }
}
