use std::ops::Deref;

use alloy::{
    primitives::{B256, Bytes},
    providers::RootProvider,
    rpc::types::engine::{ForkchoiceState, ForkchoiceUpdated, JwtSecret, PayloadStatus},
    transports::TransportResult,
};
use alloy_rpc_client::RpcClient;

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
use op_alloy_rpc_types_engine::{OpExecutionPayloadV4, OpPayloadAttributes};
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

    pub async fn new_payload_v4(
        &self,
        payload: OpExecutionPayloadV4,
        parent_beacon_block_root: B256,
    ) -> TransportResult<PayloadStatus> {
        let call = <RootProvider<Optimism> as OpEngineApi<
            Optimism,
            Http<HyperAuthClient>,
        >>::new_payload_v4(self, payload, parent_beacon_block_root);

        call.await
    }

    pub async fn fork_choice_update(
        &self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<OpPayloadAttributes>,
    ) -> TransportResult<ForkchoiceUpdated> {
        let call = <RootProvider<Optimism> as OpEngineApi<
            Optimism,
            Http<HyperAuthClient>,
        >>::fork_choice_updated_v3(self, fork_choice_state, payload_attributes);

        call.await
    }
}
