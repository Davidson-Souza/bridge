#![feature(decl_macro)]

mod api;
mod chainview;
mod node;
mod prove;
mod prover;
mod udata;

use std::sync::{Arc, Mutex};

use actix_rt::signal::ctrl_c;
use bitcoincore_rpc::{Auth, Client};

use futures::channel::mpsc::channel;
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
    std::thread::spawn(move || {
        Node::accept_connections(node);
    });
    let (sender, receiver) = channel(1024);
    // This is our implementation of the json-rpc api, it will listen for
    // incoming connections and serve some Utreexo data to clients.
    println!("Starting api");
    std::thread::spawn(|| {
        actix_rt::System::new()
            .block_on(api::create_api(sender))
            .unwrap()
    });

    let kill_signal = Arc::new(Mutex::new(false));
    let kill_signal2 = kill_signal.clone();

    // Keep the prover running in the background, it will download blocks and
    // create proofs for them as they are mined.
    println!("Running prover");
    std::thread::spawn(move || {
        actix_rt::System::new().block_on(async {
            let _ = ctrl_c().await;
            println!("Received a stop signal");
            *kill_signal.lock().unwrap() = true;
        })
    });

    prover.keep_up(kill_signal2, receiver)
}

macro_rules! subdir {
    ($path:expr) => {
        concat!(env!("HOME"), "/.bridge/", $path)
    };
}
pub(crate) use subdir;