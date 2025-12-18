use clap::Parser;
use futures::TryStreamExt as _;
use reth_exex::ExExEvent;
use reth_node_builder::Node;
use reth_optimism_cli::{Cli, chainspec::OpChainSpecParser};
use reth_optimism_node::{OpNode, args::RollupArgs};

use crate::{
    api::engine::{BasedEngineApi, BasedEngineApiServer as _},
    driver::Driver,
    exec::NoopExecutor,
};

#[derive(Parser, Debug, Clone)]
pub struct BasedOpRethArgs {
    #[command(flatten)]
    pub rollup: RollupArgs,
    #[command(flatten)]
    pub based_op: BasedOpArgs,
}

#[derive(Parser, Debug, Clone)]
pub struct BasedOpArgs {}

pub fn run() -> eyre::Result<()> {
    Cli::<OpChainSpecParser, BasedOpRethArgs>::parse().run(|builder, args| async move {
        let driver = Driver::spawn::<NoopExecutor>(todo!("Initialize driver"));

        let op_node = OpNode::new(args.rollup.clone());

        let node_handle = builder
            .with_types::<OpNode>()
            .with_components(op_node.components())
            .with_add_ons(op_node.add_ons())
            // Install the execution extension to handle canonical chain updates
            .install_exex("based-op", {
                // Get a clone of the driver handle.
                let driver = driver.clone();
                move |mut ctx| async move {
                    Ok(async move {
                        while let Some(note) = ctx.notifications.try_next().await? {
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
            .extend_rpc_modules(move |ctx| {
                // Add based engine API modules to the existing auth module.
                ctx.auth_module
                    .merge_auth_methods(BasedEngineApi::new(driver).into_rpc())
                    .expect("failed to merge modules");
                // TODO:
                // - Replace / extend the engine API
                // - Replace eth API
                Ok(())
            })
            .launch()
            .await?;

        // Run to completion
        node_handle.wait_for_node_exit().await?;

        Ok(())
    })
}
