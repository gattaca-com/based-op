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
use thiserror::Error;
use tower::{Layer, Service};
use tracing::{error, info};

#[derive(Clone, Debug)]
pub struct AuthConfig {
    /// Represents the current gateway's address
    pub gateway_address: Address,
    /// Represents the durationfor which the token will remain valid
    pub token_validity: Duration,
}

impl From<&GatewayArgs> for AuthConfig {
    fn from(args: &GatewayArgs) -> Self {
        Self { gateway_address: args.gateway_address, token_validity: Duration::from_secs(args.auth_duration * 60) }
    }
}

#[derive(Debug)]
pub struct AuthEntry {
    pub secret: JwtSecret,
    pub expires_at: SystemTime,
}

impl AuthEntry {
    // TODO: better id that doesn't leak secret data
    fn id(&self) -> B256 {
        B256::from_slice(self.secret.as_bytes())
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
        let expiry = issued_at + self.cfg.token_validity;

        let entry = Arc::new(AuthEntry { secret: secret.clone(), expires_at: expiry.clone() });

        self.entries.insert(entry.id(), entry.clone());
        info!(%challenger, "issued JWT secret");

        let expiry = expiry.duration_since(std::time::UNIX_EPOCH).expect("Time went backwards").as_secs();
        let issued_at = issued_at.duration_since(std::time::UNIX_EPOCH).expect("Time went backwards").as_secs();

        let claims = Claims { exp: Some(expiry), iat: issued_at };

        GatewayAuthentication { token: secret.encode(&claims).expect("able to encode JWT claims"), challenger }
    }

    pub fn validate(&self, token: &str) -> Result<Arc<AuthEntry>, AuthError> {
        let now = SystemTime::now();
        self.purge(now);

        for entry in self.entries.iter() {
            if entry.secret.validate(token).is_ok() {
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
        let bypass_auth = self.excluded_methods.iter().find(|method| *method == request.method_name()).is_some();

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
