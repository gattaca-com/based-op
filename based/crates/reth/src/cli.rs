use std::sync::{Arc, OnceLock};

use clap::Parser;
use futures::TryStreamExt as _;
use reth::providers::{CanonStateSubscriptions as _, providers::BlockchainProvider};
use reth_exex::ExExEvent;
use reth_node_builder::Node;
use reth_optimism_cli::{Cli, chainspec::OpChainSpecParser};
use reth_optimism_node::{OpNode, args::RollupArgs};

use crate::{
    api::{
        engine::{BasedEngineApi, BasedEngineApiServer as _},
        eth::{EthApi, EthApiServer as _},
    },
    driver::Driver,
};

#[derive(Parser, Debug, Clone)]
pub struct BasedOpRethArgs {
    #[command(flatten)]
    pub rollup: RollupArgs,
    #[command(flatten)]
    pub based_op: BasedOpArgs,
}

#[derive(Parser, Debug, Clone)]
pub struct BasedOpArgs {
    /// Whether to use the unsealed block as the "latest" state in RPC calls.
    #[arg(long)]
    pub unsealed_as_latest: bool,
}

impl BasedOpRethArgs {
    pub fn test() -> Self {
        Self { rollup: RollupArgs::default(), based_op: BasedOpArgs { unsealed_as_latest: true } }
    }
}

/// Run the based-op-reth node to completion, parsing args from the command line.
pub fn run() -> eyre::Result<()> {
    run_with_cli(Cli::<OpChainSpecParser, BasedOpRethArgs>::parse())
}

/// Run the based-op-reth node with args parsed from the provided iterator.
///
/// This is useful for testing where you want to provide args programmatically.
pub fn run_from_args<I, T>(args: I) -> eyre::Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    run_with_cli(Cli::<OpChainSpecParser, BasedOpRethArgs>::try_parse_from(args)?)
}

/// Internal helper that runs the node with a parsed CLI instance.
fn run_with_cli(cli: Cli<OpChainSpecParser, BasedOpRethArgs>) -> eyre::Result<()> {
    cli.run(|builder, args| async move {
        let driver = Arc::new(OnceLock::<Driver>::new());

        let op_node = OpNode::new(args.rollup.clone());

        let node_handle = builder
            .with_types_and_provider::<OpNode, BlockchainProvider<_>>()
            .with_components(op_node.components())
            .with_add_ons(op_node.add_ons())
            // Install the execution extension to handle canonical chain updates
            .install_exex("based-op", {
                // Get a clone of the driver handle.
                let driver = Arc::clone(&driver);
                move |mut ctx| async move {
                    Ok(async move {
                        let driver = driver.get_or_init(|| Driver::new(ctx.provider().clone()));
                        while let Some(note) = ctx.notifications.try_next().await? {
                            // TODO: Handle reorged and reverted chains?
                            if let Some(committed) = note.committed_chain() {
                                // Handle committed blocks by notifying the driver
                                for block in committed.blocks_iter() {
                                    driver.forkchoice_updated(block.clone().into_block()).await?;
                                }

                                let _ = ctx.events.send(ExExEvent::FinishedHeight(committed.tip().num_hash()));
                            }
                        }

                        Ok(())
                    })
                }
            })
            .extend_rpc_modules({
                let driver = driver.clone();
                move |ctx| {
                    let provider = ctx.provider().clone();

                    let mut canon_stream = provider.subscribe_to_canonical_state();

                    // NOTE: Not entirely sure why this is needed
                    // Ref: <https://github.com/base/node-reth/blob/7b97937fa6361f7a62f7bdb6d0cf6889c8fae87a/crates/test-utils/src/node.rs#L167-L172>
                    tokio::spawn(async move {
                        while let Ok(notif) = canon_stream.recv().await {
                            provider.canonical_in_memory_state().notify_canon_state(notif);
                        }
                    });

                    let driver = driver.get_or_init(|| Driver::new(ctx.provider().clone()));

                    // Add based engine API modules to the existing auth module.
                    ctx.auth_module.merge_auth_methods(BasedEngineApi::new(driver.clone()).into_rpc())?;

                    // Print supported engine_ methods
                    let methods = ctx
                        .auth_module
                        .module_mut()
                        .method_names()
                        .filter(|m| m.starts_with("engine_"))
                        .collect::<Vec<_>>();

                    tracing::info!(supported_methods = ?methods, "Configured based engine API");

                    // Configure extended Eth API
                    let eth = EthApi {
                        canonical: ctx.registry.eth_api().clone(),
                        eth_filter: ctx.registry.eth_handlers().filter.clone(),
                        unsealed_state: (),
                        unsealed_as_latest: args.based_op.unsealed_as_latest,
                    };

                    ctx.modules.replace_configured(eth.into_rpc())?;

                    Ok(())
                }
            })
            .launch()
            .await?;

        // Run to completion
        node_handle.wait_for_node_exit().await?;

        Ok(())
    })
}
