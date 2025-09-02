//! An engine client.

use alloy::{
    primitives::{B256, Bytes},
    providers::{Provider, RootProvider},
    rpc::types::{
        SyncStatus,
        engine::{
            ExecutionPayloadInputV2, ForkchoiceState, ForkchoiceUpdated, JwtSecret, PayloadStatus,
        },
    },
    transports::TransportResult,
};
use alloy_rpc_client::RpcClient;
use std::{ops::Deref, time::Duration};

use alloy_transport_http::{
    AuthLayer, AuthService, Http, HyperClient,
    hyper_util::{
        client::legacy::{Client, connect::HttpConnector},
        rt::TokioExecutor,
    },
};
use http_body_util::Full;
use op_alloy_network::Optimism;
use op_alloy_provider::ext::engine::OpEngineApi;
use op_alloy_rpc_types_engine::{OpExecutionPayload, OpPayloadAttributes};
use tokio::time::sleep;
use tower::ServiceBuilder;
use url::Url;

/// A Hyper HTTP client with a JWT authentication layer.
type HyperAuthClient<B = Full<Bytes>> = HyperClient<B, AuthService<Client<HttpConnector, B>>>;

#[derive(Debug, Clone)]
pub struct EngineClient {
    inner: RootProvider<Optimism>,
}

impl Deref for EngineClient {
    type Target = RootProvider<Optimism>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl EngineClient {
    pub fn new(url: Url, secret: JwtSecret) -> Self {
        let hyper_client = Client::builder(TokioExecutor::new()).build_http::<Full<Bytes>>();
        let auth_layer = AuthLayer::new(secret);
        let service = ServiceBuilder::new()
            .layer(auth_layer)
            .service(hyper_client);
        let layer_transport = HyperClient::with_service(service);

        let http_hyper = Http::with_client(layer_transport, url);
        let rpc_client = RpcClient::new(http_hyper, false);
        let inner = RootProvider::<Optimism>::new(rpc_client);

        Self { inner }
    }

    pub async fn new_payload(
        &self,
        payload: OpExecutionPayload,
        parent_beacon_block_root: Option<B256>,
    ) -> TransportResult<PayloadStatus> {
        let call = match payload {
            OpExecutionPayload::V1(v1) => {
                let input = ExecutionPayloadInputV2 {
                    execution_payload: v1,
                    withdrawals: None,
                };
                // It is also intended to use v2 but with nil withdrawals, as new_payload_v1 isn't
                // exposed.
                <RootProvider<Optimism> as OpEngineApi<
                Optimism,
                Http<HyperAuthClient>,
            >>::new_payload_v2(self, input)
            }
            OpExecutionPayload::V2(v2) => <RootProvider<Optimism> as OpEngineApi<
                Optimism,
                Http<HyperAuthClient>,
            >>::new_payload_v2(
                self, v2.into_payload_input_v2(true)
            ),
            OpExecutionPayload::V3(v3) => <RootProvider<Optimism> as OpEngineApi<
                Optimism,
                Http<HyperAuthClient>,
            >>::new_payload_v3(
                self,
                v3,
                parent_beacon_block_root.expect("parent_beacon_block_root"),
            ),
            OpExecutionPayload::V4(v4) => <RootProvider<Optimism> as OpEngineApi<
                Optimism,
                Http<HyperAuthClient>,
            >>::new_payload_v4(
                self,
                v4,
                parent_beacon_block_root.expect("parent_beacon_block_root"),
            ),
        };

        call.await
    }

    pub async fn fork_choice_update(
        &self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<OpPayloadAttributes>,
        is_v3: bool,
    ) -> TransportResult<ForkchoiceUpdated> {
        let call = if is_v3 {
            <RootProvider<Optimism> as OpEngineApi<
            Optimism,
            Http<HyperAuthClient>,
        >>::fork_choice_updated_v3(self, fork_choice_state, payload_attributes)
        } else {
            <RootProvider<Optimism> as OpEngineApi<
            Optimism,
            Http<HyperAuthClient>,
        >>::fork_choice_updated_v2(self, fork_choice_state, payload_attributes)
        };

        call.await
    }

    /// Waits for the EL client to be synced.
    pub async fn wait_for_sync(&self, poll_time: Duration) -> TransportResult<()> {
        loop {
            let sync_info = self.syncing().await?;
            match sync_info {
                SyncStatus::None => return Ok(()),
                SyncStatus::Info(info) => {
                    println!("Syncing: {info:?}");
                }
            }
            sleep(poll_time).await;
        }
    }
}
