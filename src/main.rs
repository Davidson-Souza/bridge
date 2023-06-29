mod prove;
mod prover;
mod udata;

use std::sync::Arc;

use bitcoincore_rpc::{Auth, Client};

use prove::{ProofFile, ProofsIndex};
fn main() {
    let cookie = env!("HOME").to_owned() + "/.bitcoin/signet/.cookie";
    let client = Client::new("localhost:38332".into(), Auth::CookieFile(cookie.into())).unwrap();
    let index_store = ProofsIndex {
        database: kv::Store::new(kv::Config {
            path: "./index/".into(),
            temporary: false,
            use_compression: false,
            flush_every_ms: None,
            cache_capacity: None,
            segment_size: None,
        })
        .unwrap()
        .bucket(Some("proofs"))
        .unwrap(),
    };
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .read(true)
        .open("test.proofs")
        .unwrap();
    let file = ProofFile::new(file);
    let mut prover = prover::Prover::new(client, Arc::new(index_store), file);
    prover.prove_range(0, 10_000);
}
