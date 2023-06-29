mod blockstore;
mod node;
mod prove;
mod prover;
mod udata;

use std::sync::Arc;

use bitcoincore_rpc::{Auth, Client, RpcApi};

use prove::{ProofFile, ProofFileManager, ProofsIndex};
fn main() {
    let cookie = env!("HOME").to_owned() + "/.bitcoin/signet/.cookie";
    let client = Client::new(
        "localhost:38332".into(),
        Auth::CookieFile(cookie.clone().into()),
    )
    .unwrap();
    let index_store = ProofsIndex {
        database: kv::Store::new(kv::Config {
            path: "./index/".into(),
            temporary: false,
            use_compression: false,
            flush_every_ms: None,
            cache_capacity: None,
            segment_size: None,
        })
        .unwrap(),
    };
    let index_store = Arc::new(index_store);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .read(true)
        .open("test.proofs")
        .unwrap();
    let file = ProofFile::new(file);
    let mut prover = prover::Prover::new(client, index_store.clone(), file);
    // prover.prove_range(0, 10_000);
    let listener = std::net::TcpListener::bind("127.0.0.1:8333").unwrap();
    let client = Client::new("localhost:38332".into(), Auth::CookieFile(cookie.into())).unwrap();
    let client = Arc::new(client);
    let block_storage = Arc::new(blockstore::BlockStore::new("./blocks"));
    println!("Downloading blocks");
    let chain_info = client.get_blockchain_info().unwrap();
    for i in 149489..=chain_info.blocks {
        let block = client.get_block_hash(i).unwrap();
        let block = client.get_block(&block).unwrap();
        block_storage.put_block(&block);
    }

    println!("Starting p2p node");
    let node = node::Node::new(
        listener,
        Arc::new(ProofFileManager::new()),
        index_store,
        client,
        block_storage,
    );
    node.accept_connections();
}
