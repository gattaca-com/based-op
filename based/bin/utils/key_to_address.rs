use std::str::FromStr;

use bop_common::signing::ECDSASigner;
use revm_primitives::B256;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <private-key>", args[0]);
        std::process::exit(1);
    }
    let address = ECDSASigner::new(B256::from_str(&args[1]).expect("wrong key format")).expect("wrong key format").address;
    println!("{}", address)
}
