use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime},
};

use alloy_primitives::{Address, B256, Signature};
use alloy_rpc_types::engine::{Claims, JwtSecret};
use bop_common::{
    api::{BasedAuthApiServer, GatewayAuthentication},
    auth::gateway_auth_message,
    communication::messages::{RpcError, RpcResult},
    config::GatewayArgs,
};
use dashmap::DashMap;
use http::{HeaderMap, StatusCode};
use jsonrpsee::{
    MethodResponse, RpcModule,
    core::{
        async_trait,
        middleware::{Request, RpcServiceT},
    },
    http_client::{HttpBody, HttpRequest, HttpResponse},
    types::{ErrorObject, error::INVALID_PARAMS_CODE},
};
use jsonwebtoken::{EncodingKey, errors::ErrorKind};
use reth_rpc_layer::JwtError;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tower::{Layer, Service};
use tracing::info;

#[derive(Clone, Debug)]
pub struct AuthConfig {
    /// Represents the current gateway's address
    pub gateway_address: Address,
    /// Represents the durationfor which the token will remain valid
    pub token_validity: Duration,
}

impl AuthConfig {
    pub fn new(address: Address, token_duration_secs: u64) -> Self {
        Self { gateway_address: address, token_validity: Duration::from_secs(token_duration_secs) }
    }
}

#[derive(Debug)]
pub struct AuthEntry {
    pub secret: Vec<u8>,
    pub expires_at: SystemTime,
}

impl AuthEntry {
    // TODO: better id that doesn't leak secret data
    fn id(&self) -> B256 {
        B256::from_slice(&self.secret)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TokenClaims {
    /// Issued at UNIX timestamp
    issued_at: u64,
    /// The expiration UNIX timestamp
    expiry: u64,
}

impl TokenClaims {
    pub fn from_systemtime(iat: SystemTime, exp: SystemTime) -> Self {
        debug_assert!(iat < exp, "issuance should always be before expiry");

        let issued_at = iat.duration_since(std::time::UNIX_EPOCH).expect("Time went backwards").as_secs();
        let expiry = exp.duration_since(std::time::UNIX_EPOCH).expect("Time went backwards").as_secs();

        Self { issued_at, expiry }
    }

    const fn signature_algo() -> jsonwebtoken::Algorithm {
        jsonwebtoken::Algorithm::HS256
    }

    fn is_within_time_window(&self) -> bool {
        let now = jsonwebtoken::get_current_timestamp();
        self.issued_at <= now && self.expiry >= now
    }

    pub fn encode(&self, secret: &[u8]) -> Result<String, jsonwebtoken::errors::Error> {
        let secret = jsonwebtoken::EncodingKey::from_secret(secret);
        let algo = jsonwebtoken::Header::new(Self::signature_algo());
        jsonwebtoken::encode(&algo, self, &secret)
    }

    pub fn validate(token: &str, secret: &[u8]) -> Result<(), JwtError> {
        let mut validation = jsonwebtoken::Validation::new(Self::signature_algo());
        validation.set_required_spec_claims(&["iat"]);

        match jsonwebtoken::decode::<Self>(token, &jsonwebtoken::DecodingKey::from_secret(secret), &validation) {
            Ok(token) => {
                if !token.claims.is_within_time_window() {
                    Err(JwtError::InvalidIssuanceTimestamp)?
                }

                Ok(())
            }
            Err(err) => match *err.kind() {
                ErrorKind::InvalidSignature => Err(JwtError::InvalidSignature)?,
                ErrorKind::InvalidAlgorithm => Err(JwtError::UnsupportedSignatureAlgorithm)?,
                _ => {
                    let detail = format!("{err}");
                    Err(JwtError::JwtDecodingError(detail))?
                }
            },
        }
    }
}

#[derive(Debug)]
pub struct AuthManager {
    entries: DashMap<B256, Arc<AuthEntry>>,
    cfg: AuthConfig,
}

impl AuthManager {
    pub fn new(cfg: AuthConfig) -> Self {
        Self { entries: DashMap::new(), cfg }
    }

    pub fn config(&self) -> &AuthConfig {
        &self.cfg
    }

    fn purge(&self, now: SystemTime) {
        self.entries.retain(|_, entry| entry.expires_at >= now);
    }

    pub fn issue(&self, challenger: Address, issued_at: SystemTime) -> GatewayAuthentication {
        let secret = JwtSecret::random();
        let secret = secret.as_bytes().to_vec();

        let expiry = issued_at + self.cfg.token_validity;

        let entry = Arc::new(AuthEntry { secret: secret.clone(), expires_at: expiry });

        self.entries.insert(entry.id(), entry.clone());
        tracing::debug!(%challenger, "issued JWT secret");

        let claims = TokenClaims::from_systemtime(issued_at, expiry);

        GatewayAuthentication { token: claims.encode(&secret).expect("able to encode JWT claims"), challenger }
    }

    pub fn validate(&self, token: &str) -> Result<Arc<AuthEntry>, AuthError> {
        let now = SystemTime::now();
        self.purge(now);

        for entry in self.entries.iter() {
            if TokenClaims::validate(token, &entry.secret).is_ok() {
                return Ok(entry.clone());
            }
        }

        Err(AuthError::UnknownToken)
    }
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("token did not match any active secret")]
    UnknownToken,
}

/// Produced by [`GatewayAuthLayer`] to indicate if the token was valid or not
#[derive(Clone)]
enum Authentication {
    Valid,
    Invalid,
}

/// Produced by [`GatewayRPCAuthLayer`] to indicate if the method was allowed or not
#[derive(Clone, Copy)]
enum AuthenticationResult {
    Allowed,
    Disallowed,
}

/// Interprets the JWT provided in the request's authorization header
///
/// Companion to [`GatewayRPCAuthLayer`]
pub struct GatewayAuthLayer {
    manager: Arc<AuthManager>,
}

impl GatewayAuthLayer {
    pub fn new(manager: Arc<AuthManager>) -> Self {
        Self { manager }
    }
}

impl<S> Layer<S> for GatewayAuthLayer
where
    S: Service<HttpRequest, Response = HttpResponse>,
    S::Future: Send + 'static,
{
    type Service = GatewayAuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        GatewayAuthService { manager: self.manager.clone(), inner }
    }
}

#[derive(Clone)]
pub struct GatewayAuthService<S> {
    manager: Arc<AuthManager>,
    inner: S,
}

impl<S> Service<HttpRequest> for GatewayAuthService<S>
where
    S: Service<HttpRequest, Response = HttpResponse>,
    S::Future: Send + 'static,
{
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse, Self::Error>> + Send>>;
    type Response = HttpResponse;

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, mut req: HttpRequest) -> Self::Future {
        let auth = match extract_bearer(req.headers()) {
            None => Authentication::Invalid,
            Some(token) => {
                if self.manager.validate(&token).is_err() {
                    Authentication::Invalid
                } else {
                    Authentication::Valid
                }
            }
        };

        // Add authentication result as request extension
        req.extensions_mut().insert(auth);

        let fut = self.inner.call(req);

        let fut = async move {
            fut.await.map(|response| {
                let ext = response.extensions();

                let Some(auth) = ext.get::<AuthenticationResult>() else {
                    // RPC layer not used, ignore
                    return response;
                };

                if matches!(auth, AuthenticationResult::Disallowed) {
                    err_response("Method requires authentication".to_string())
                } else {
                    response
                }
            })
        };

        Box::pin(fut)
    }
}

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    let header = headers.get(http::header::AUTHORIZATION)?;
    let auth = header.to_str().ok()?;
    auth.strip_prefix("Bearer ").map(str::to_owned)
}

fn err_response(err: String) -> HttpResponse {
    HttpResponse::builder()
        .status(StatusCode::UNAUTHORIZED)
        .body(HttpBody::new(err))
        .expect("failed to build unauthorized response")
}

/// Enforces authentication for the RPC methods NOT in `excluded_methods`
///
/// Companion to [`GatewayAuthLayer`]
#[derive(Clone)]
pub struct GatewayRPCAuthLayer {
    excluded_methods: Arc<Box<[String]>>,
}

impl GatewayRPCAuthLayer {
    pub fn new(excluded_method_names: Arc<Box<[String]>>) -> Self {
        Self { excluded_methods: excluded_method_names }
    }

    pub fn exclude<T>(module: &RpcModule<T>) -> Self {
        let method_names = module.method_names().map(|s| s.to_owned()).collect::<Vec<_>>().into_boxed_slice();
        Self::new(Arc::new(method_names))
    }
}

impl<S> Layer<S> for GatewayRPCAuthLayer {
    type Service = GatewayRPCAuth<S>;

    fn layer(&self, inner: S) -> Self::Service {
        GatewayRPCAuth { excluded_methods: self.excluded_methods.clone(), inner }
    }
}

pub struct GatewayRPCAuth<S> {
    excluded_methods: Arc<Box<[String]>>,
    inner: S,
}

impl<S> RpcServiceT for GatewayRPCAuth<S>
where
    S: RpcServiceT<MethodResponse = MethodResponse>,
{
    type BatchResponse = S::BatchResponse;
    type MethodResponse = MethodResponse;
    type NotificationResponse = S::NotificationResponse;

    fn call<'a>(&self, request: Request<'a>) -> impl Future<Output = Self::MethodResponse> + Send + 'a {
        let auth = request.extensions().get::<Authentication>();

        // bypass auth for the given method if the method is excluded
        let bypass_auth = self.excluded_methods.iter().any(|method| method == request.method_name());

        let authentication_result = match auth {
            _ if bypass_auth => AuthenticationResult::Allowed,
            Some(Authentication::Valid) => AuthenticationResult::Allowed,
            _ => AuthenticationResult::Disallowed,
        };

        let id = request.id().clone();
        let fut = self.inner.call(request);

        async move {
            let mut response = if matches!(authentication_result, AuthenticationResult::Allowed) {
                fut.await
            } else {
                MethodResponse::error(
                    id,
                    ErrorObject::borrowed(INVALID_PARAMS_CODE, "Method requires authentication", None),
                )
            };
            response.extensions_mut().insert(authentication_result);

            response
        }
    }

    fn batch<'a>(
        &self,
        requests: jsonrpsee::core::middleware::Batch<'a>,
    ) -> impl Future<Output = Self::BatchResponse> + Send + 'a {
        self.inner.batch(requests)
    }

    fn notification<'a>(
        &self,
        n: jsonrpsee::core::middleware::Notification<'a>,
    ) -> impl Future<Output = Self::NotificationResponse> + Send + 'a {
        self.inner.notification(n)
    }
}

#[derive(Clone)]
pub struct AuthRpc {
    manager: Arc<AuthManager>,
}

impl AuthRpc {
    pub fn new(manager: Arc<AuthManager>) -> Self {
        Self { manager }
    }

    pub fn http_layer(&self) -> GatewayAuthLayer {
        GatewayAuthLayer::new(self.manager.clone())
    }

    pub fn rpc_layer(&self) -> GatewayRPCAuthLayer {
        let module = BasedAuthApiServer::into_rpc(self.clone());
        GatewayRPCAuthLayer::exclude(&module)
    }
}

#[async_trait]
impl BasedAuthApiServer for AuthRpc {
    async fn authentication_challenge(&self, valid_from: u64) -> RpcResult<B256> {
        Ok(gateway_auth_message(self.manager.config().gateway_address, valid_from))
    }

    async fn authenticate_proposer(&self, valid_from: u64, signature: Signature) -> RpcResult<GatewayAuthentication> {
        let payload_hash = self.authentication_challenge(valid_from).await?;
        let challenger = signature
            .recover_address_from_prehash(&payload_hash)
            .map_err(|_| RpcError::Generic("invalid signature"))?;

        let valid_from = Duration::from_secs(valid_from);
        let valid_from = SystemTime::UNIX_EPOCH + valid_from;

        // TODO: add authorization logic, verifying challenger may authenticate with this gateway
        Ok(self.manager.issue(challenger, valid_from))
    }
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, sync::Arc, time::SystemTime};

    use bop_common::{
        api::{BasedAuthApiServer, ControlApiServer},
        auth::gateway_auth_message,
        communication::messages::RpcResult,
        signing::ECDSASigner,
    };
    use jsonrpsee::server::{ServerBuilder, ServerHandle, middleware::rpc::RpcServiceBuilder};
    use serde_json::{Value, json};

    use super::{AuthConfig, AuthManager, AuthRpc};

    #[derive(Clone)]
    struct TestControlRpc;

    #[jsonrpsee::core::async_trait]
    impl ControlApiServer for TestControlRpc {
        async fn heartbeat(&self) -> RpcResult<()> {
            Ok(())
        }
    }

    async fn post_jsonrpc(url: &str, body: Value, auth_header: Option<&str>) -> (reqwest::StatusCode, String) {
        // We do the JSON-RPC request manually (instead of using jsonrpsee) to check the HTTP status code returned.
        let client = reqwest::Client::new();
        let mut req = client.post(url).json(&body);
        if let Some(value) = auth_header {
            req = req.header(reqwest::header::AUTHORIZATION, value);
        }
        let resp = req.send().await.expect("request should succeed");
        let status = resp.status();
        let text = resp.text().await.expect("response body should be readable");
        (status, text)
    }

    struct StartedServer {
        url: String,
        handle: ServerHandle,
        gateway_address: alloy_primitives::Address,
    }

    async fn start_server() -> StartedServer {
        let gateway_address = alloy_primitives::Address::from([0x11u8; 20]);
        let auth_manager = Arc::new(AuthManager::new(AuthConfig {
            gateway_address,
            token_validity: std::time::Duration::from_secs(60),
        }));

        let auth_rpc = AuthRpc::new(auth_manager.clone());

        let http_middleware = tower::ServiceBuilder::new().layer(auth_rpc.http_layer());
        let rpc_middleware = RpcServiceBuilder::new().layer(auth_rpc.rpc_layer());

        let server = ServerBuilder::default()
            .set_http_middleware(http_middleware)
            .set_rpc_middleware(rpc_middleware)
            .build(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .expect("server should build");

        let addr = server.local_addr().expect("server should have local addr");
        let url = format!("http://{addr}");

        let mut module = ControlApiServer::into_rpc(TestControlRpc);
        module.merge(BasedAuthApiServer::into_rpc(auth_rpc)).expect("failed to merge based auth rpc");

        let handle = server.start(module);

        StartedServer { url, handle, gateway_address }
    }

    fn get_jsonrpc_success(body: &str) -> Option<Value> {
        let value: Value = serde_json::from_str(body).ok()?;
        if value.get("error").is_some() {
            return None;
        }
        value.get("result").cloned()
    }

    #[tokio::test]
    async fn rejects_unauthenticated_requests_for_non_excluded_methods() {
        let StartedServer { url, handle, .. } = start_server().await;

        // Without a token, non-excluded methods are rejected at the HTTP layer.
        let (status, body) =
            post_jsonrpc(&url, json!({"jsonrpc":"2.0","id":1,"method":"control_heartbeat","params":[]}), None).await;
        assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);
        assert!(body.contains("Method requires authentication"), "unexpected body: {body}");

        // With a fake token, non-excluded methods are still rejected.
        let (status, body) = post_jsonrpc(
            &url,
            json!({"jsonrpc":"2.0","id":2,"method":"control_heartbeat","params":[]}),
            Some("Bearer totally-fake"),
        )
        .await;
        assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);
        assert!(body.contains("Method requires authentication"), "unexpected body: {body}");

        handle.stop().expect("server stop should succeed");
        handle.stopped().await;
    }

    #[tokio::test]
    async fn allows_excluded_based_auth_methods_without_token() {
        let StartedServer { url, handle, gateway_address } = start_server().await;

        // Auth methods are excluded from enforcement (allowlist).
        let valid_from = SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("time ok").as_secs();
        let (status, body) = post_jsonrpc(
            &url,
            json!({"jsonrpc":"2.0","id":3,"method":"based_authenticationChallenge","params":[valid_from]}),
            None,
        )
        .await;
        assert_eq!(status, reqwest::StatusCode::OK);
        let challenge_value = get_jsonrpc_success(&body).expect("expected jsonrpc success");
        let challenge: alloy_primitives::B256 =
            serde_json::from_value(challenge_value).expect("challenge should be B256");
        assert_eq!(challenge, gateway_auth_message(gateway_address, valid_from));

        handle.stop().expect("server stop should succeed");
        handle.stopped().await;
    }

    #[tokio::test]
    async fn token_from_authenticate_proposer_allows_followup_requests_with_bearer_prefix() {
        let StartedServer { url, handle, gateway_address } = start_server().await;

        let valid_from = SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("time ok").as_secs();

        // Obtain a real token via based_authenticateProposer.
        let signer = ECDSASigner::random();
        let signature =
            signer.sign_message(gateway_auth_message(gateway_address, valid_from)).expect("signature should succeed");

        let (status, body) = post_jsonrpc(
            &url,
            json!({"jsonrpc":"2.0","id":4,"method":"based_authenticateProposer","params":[valid_from, signature]}),
            None,
        )
        .await;
        assert_eq!(status, reqwest::StatusCode::OK);
        let auth_value = get_jsonrpc_success(&body).expect("expected jsonrpc success");
        let token = auth_value.get("token").and_then(|v| v.as_str()).expect("token should be string").to_owned();

        // Token without "Bearer " prefix is not interpreted as a bearer token.
        let (status, body) =
            post_jsonrpc(&url, json!({"jsonrpc":"2.0","id":5,"method":"control_heartbeat","params":[]}), Some(&token))
                .await;
        assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);
        assert!(body.contains("Method requires authentication"), "unexpected body: {body}");

        // Token with "Bearer " prefix allows access to the rest of the gateway RPC methods.
        let (status, body) = post_jsonrpc(
            &url,
            json!({"jsonrpc":"2.0","id":6,"method":"control_heartbeat","params":[]}),
            Some(&format!("Bearer {token}")),
        )
        .await;
        assert_eq!(status, reqwest::StatusCode::OK);
        get_jsonrpc_success(&body).expect("expected jsonrpc success");
        handle.stop().expect("server stop should succeed");
        handle.stopped().await;
    }
}
