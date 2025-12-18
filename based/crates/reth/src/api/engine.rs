use bop_common::p2p::{EnvV0, FragV0, SealV0, Signed};
use jsonrpsee::{
    core::{RpcResult, async_trait},
    proc_macros::rpc,
    types::{ErrorCode, ErrorObject},
};

use crate::{driver::Driver, error::DriverError};

#[rpc(server, namespace = "engine")]
pub trait BasedEngineApi {
    #[method(name = "envV0")]
    async fn env_v0(&self, env: Signed<EnvV0>) -> RpcResult<()>;

    #[method(name = "newFragV0")]
    async fn new_frag_v0(&self, frag: Signed<FragV0>) -> RpcResult<()>;

    #[method(name = "sealFragV0")]
    async fn seal_frag_v0(&self, seal: Signed<SealV0>) -> RpcResult<()>;
}

pub struct BasedEngineApi {
    driver: Driver,
}

impl BasedEngineApi {
    /// Initialize a new based engine API instance.
    pub fn new(driver: Driver) -> Self {
        Self { driver }
    }
}

#[async_trait]
impl BasedEngineApiServer for BasedEngineApi {
    #[tracing::instrument(skip(self))]
    async fn env_v0(&self, env: Signed<EnvV0>) -> RpcResult<()> {
        tracing::debug!("handling engine_envV0");

        self.driver.env_v0(env.message).await?;
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn new_frag_v0(&self, frag: Signed<FragV0>) -> RpcResult<()> {
        tracing::debug!("handling engine_newFragV0");

        self.driver.new_frag_v0(frag.message).await?;
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn seal_frag_v0(&self, seal: Signed<SealV0>) -> RpcResult<()> {
        tracing::debug!("handling engine_sealFragV0");

        self.driver.seal_frag_v0(seal.message).await?;
        Ok(())
    }
}

impl From<DriverError> for ErrorObject<'static> {
    // TODO: Better error handling
    fn from(e: DriverError) -> Self {
        ErrorObject::owned(ErrorCode::InternalError.code(), e.to_string(), Option::<()>::None)
    }
}
