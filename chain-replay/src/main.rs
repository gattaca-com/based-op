use clap::Parser;

use crate::config::Args;

mod config;

fn main() {
    let args = Args::parse().validate();
}
