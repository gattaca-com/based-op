use bop_common::utils::init_tracing;
use tracing::info;

fn main() {
    let _guard = init_tracing(None);

    info!("Hello, world!");
}
