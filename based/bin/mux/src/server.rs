use std::{net::SocketAddr, time::Duration};

use alloy_primitives::B256;
use alloy_rpc_types::engine::{ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, PayloadId, PayloadStatus};
use bop_common::{
    api::{EngineApiClient, EngineApiServer, CAPABILITIES},
    communication::messages::{RpcError, RpcResult},
    utils::wait_for_signal,
};
use jsonrpsee::{
    core::async_trait,
    http_client::{transport::HttpBackend, HttpClientBuilder},
    server::{RpcServiceBuilder, ServerBuilder},
};
use op_alloy_rpc_types_engine::{OpExecutionPayloadEnvelopeV3, OpPayloadAttributes};
use reth_rpc_layer::{AuthClientLayer, AuthClientService};
use tracing::{debug, error, info, trace, Instrument, Level};

use crate::{cli::MuxArgs, middleware::ProxyService};

pub type HttpClient = jsonrpsee::http_client::HttpClient<AuthClientService<HttpBackend>>;

pub struct MuxServer {
    fallback_client: HttpClient,
    gateway_client: HttpClient,
}

impl MuxServer {
    pub fn new(args: MuxArgs) -> eyre::Result<Self> {
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
        info!(%addr, "starting RPC server");

        let fallback_client = self.fallback_client.clone();
        let rpc_middleware =
            RpcServiceBuilder::new().layer_fn(move |s| ProxyService::new(CAPABILITIES, s, fallback_client.clone()));

        let server = ServerBuilder::default().set_rpc_middleware(rpc_middleware).build(addr).await?;
        let execution_module = EngineApiServer::into_rpc(self);

        let server_handle = server.start(execution_module);

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

#[async_trait]
impl EngineApiServer for MuxServer {
    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn fork_choice_updated_v3(
        &self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<OpPayloadAttributes>,
    ) -> RpcResult<ForkchoiceUpdated> {
        trace!(?fork_choice_state, ?payload_attributes, "new request");

        // send in background to gateway
        // return what fallback returns

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
                        Err(err) => {
                            error!(%err, "failed gateway");
                        }
                    }
                }
            }
            .in_current_span(),
        );

        let response = self.fallback_client.fork_choice_updated_v3(fork_choice_state, payload_attributes).await?;

        Ok(response)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn new_payload_v3(
        &self,
        payload: ExecutionPayloadV3,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
    ) -> RpcResult<PayloadStatus> {
        trace!(?payload, ?versioned_hashes, %parent_beacon_block_root, "new request");

        // send in background to gateway
        // return what fallback returns

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
                        Err(err) => {
                            error!(%err, "failed gateway");
                        }
                    }
                }
            }
            .in_current_span(),
        );

        let response = self.fallback_client.new_payload_v3(payload, versioned_hashes, parent_beacon_block_root).await?;

        Ok(response)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn get_payload_v3(&self, payload_id: PayloadId) -> RpcResult<OpExecutionPayloadEnvelopeV3> {
        trace!(%payload_id, "new request");

        let fallback_fut = tokio::spawn({
            let client = self.fallback_client.clone();

            async move { client.get_payload_v3(payload_id).await }
        });

        let gateway_fut: tokio::task::JoinHandle<Result<OpExecutionPayloadEnvelopeV3, _>> = tokio::spawn(
            {
                let gateway_client = self.gateway_client.clone();
                let fallback_client = self.fallback_client.clone();

                async move {
                    let gateway_payload = gateway_client.get_payload_v3(payload_id).await?;
                    let payload_status = fallback_client
                        .new_payload_v3(
                            gateway_payload.execution_payload.clone(),
                            vec![],
                            gateway_payload.parent_beacon_block_root,
                        )
                        .await?;

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
