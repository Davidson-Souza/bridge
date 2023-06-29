mod chainview;
mod node;
mod prove;
mod prover;
mod udata;
use std::sync::{Arc, Mutex};

use bitcoincore_rpc::{Auth, Client};

use prove::{BlocksFileManager, BlocksIndex};

use crate::node::Node;

fn main() -> anyhow::Result<()> {
    // Create a json-rpc client to bitcoin core
    let cookie = env!("HOME").to_owned() + "/.bitcoin/signet/.cookie";
    let client = Client::new(
        "localhost:38332".into(),
        Auth::CookieFile(cookie.clone().into()),
    )
    .unwrap();
    // Create a chainview, this module will download headers from the bitcoin core
    // to keep track of the current chain state and speed up replying to headers requests
    // from peers.
    let store = kv::Store::new(kv::Config {
        path: subdir!("chain_view").into(),
        temporary: false,
        use_compression: false,
        flush_every_ms: None,
        cache_capacity: None,
        segment_size: None,
    })
    .expect("Failed to open chainview database");
    let view = chainview::ChainView::new(store);
    let view = Arc::new(view);

    // This database stores some useful information about the blocks, but not
    // the blocks themselves
    let index_store = BlocksIndex {
        database: kv::Store::new(kv::Config {
            path: subdir!("index/").into(),
            temporary: false,
            use_compression: false,
            flush_every_ms: None,
            cache_capacity: None,
            segment_size: None,
        })
        .unwrap(),
    };
    // Put it into an Arc so we can share it between threads
    let index_store = Arc::new(index_store);
    // This database stores the blocks themselves, it's a collection of flat files
    // that are indexed by the index above. They are stored in the `blocks/` directory
    // and are serialized as bitcoin blocks, so we don't need to do any parsing
    // before sending to a peer.
    let blocks = Arc::new(Mutex::new(BlocksFileManager::new()));
    // Create a prover, this module will download blocks from the bitcoin core
    // node and save them to disk. It will also create proofs for the blocks
    // and save them to disk.
    let mut prover = prover::Prover::new(client, index_store.clone(), blocks.clone(), view.clone());

    println!("Starting p2p node");
    // This is our implementation of the Bitcoin p2p protocol, it will listen
    // for incoming connections and serve blocks and proofs to peers.
    let listener = std::net::TcpListener::bind("127.0.0.1:8333").unwrap();
    let node = node::Node::new(listener, blocks, index_store, view);
    std::thread::spawn(|| Node::accept_connections(node));
    // Keep the prover running in the background, it will download blocks and
    // create proofs for them as they are mined.
    prover.keep_up()
}

macro_rules! subdir {
    ($path:expr) => {
        concat!(env!("HOME"), "/.bridge/", $path)
    };
}
pub(crate) use subdir;
