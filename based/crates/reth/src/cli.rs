use clap::Parser;
use futures::TryStreamExt as _;
use reth_exex::ExExEvent;
use reth_optimism_cli::{Cli, chainspec::OpChainSpecParser};
use reth_optimism_node::{OpNode, args::RollupArgs};

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
    Cli::<OpChainSpecParser, BasedOpRethArgs>::parse().run(|builder, mut args| async move {
        let op_node = OpNode::new(args.rollup.clone());

        let add_ons = op_node.add_ons_builder().build();

        let node_handle = builder
            .with_types::<OpNode>()
            .with_components(op_node.components())
            .with_add_ons(add_ons)
            // Install the execution extension to handle canonical chain updates
            .install_exex("based-op", {
                move |mut ctx| async move {
                    Ok(async move {
                        while let Some(note) = ctx.notifications.try_next().await? {
                            if let Some(committed) = note.committed_chain() {
                                for b in committed.blocks_iter() {
                                    // TODO: Handle committed block
                                }
                                let _ = ctx.events.send(ExExEvent::FinishedHeight(committed.tip().num_hash()));
                            }
                        }

                        Ok(())
                    })
                }
            })
            // .extend_rpc_modules(move |rpx_context| {
            //     // WTF
            //     Ok(())
            // })
            .launch()
            .await?;

        // TODO
        Ok(())
    })
}
