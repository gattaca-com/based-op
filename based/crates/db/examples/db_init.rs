fn main() {
    let db_dir = "~/state_trie_db";
    let db = bop_db::init_database(db_dir).expect("Failed to init DB");
    println!("loaded DB: {db:?}");
}
