use std::str::FromStr;

use bop_common::signing::ECDSASigner;
use revm_primitives::B256;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let private_key = if args.len() >= 2 {
        B256::from_str(&args[1]).expect("wrong key format")
    } else if args.len() == 1{
        B256::random()
    } else{
        panic!("Don't know how to handle {} args", args.len())
    };

    let address =
            ECDSASigner::new(private_key).expect("wrong key format").address;

    println!("{}", private_key);
    println!("{}", address);
}
