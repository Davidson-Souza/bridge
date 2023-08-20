//SPDX-License-Identifier: MIT

mod api;
mod chaininterface;
mod chainview;
#[cfg(feature = "esplora")]
mod esplora;
mod node;
mod prove;
mod prover;
mod udata;

use actix_rt::signal::ctrl_c;
use anyhow::Result;
use bitcoincore_rpc::{Auth, Client};
use std::{
    env, fs,
    sync::{Arc, Mutex},
};

use chaininterface::Blockchain;
use futures::channel::mpsc::channel;
use log::{info, warn};
use prove::{BlocksFileManager, BlocksIndex};
use simplelog::{Config, SharedLogger};

use crate::node::Node;

fn main() -> anyhow::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .create(subdir(""))
        .unwrap();

    // Initialize the logger
    init_logger(
        Some(&subdir("debug.log")),
        simplelog::LevelFilter::Info,
        true,
    );
    // let client = esplora::EsploraBlockchain::new("https://mempool.space/signet/api".into());
    // Create a chainview, this module will download headers from the bitcoin core
    // to keep track of the current chain state and speed up replying to headers requests
    // from peers.
    let store = kv::Store::new(kv::Config {
        path: subdir("chain_view").into(),
        temporary: false,
        use_compression: false,
        flush_every_ms: None,
        cache_capacity: None,
        segment_size: None,
    })
    .expect("Failed to open chainview database");
    // Chainview is a collection of metadata about the chain, like tip and block
    // indexes. It's stored in a key-value database.
    let view = chainview::ChainView::new(store);
    let view = Arc::new(view);

    // This database stores some useful information about the blocks, but not
    // the blocks themselves
    let index_store = BlocksIndex {
        database: kv::Store::new(kv::Config {
            path: subdir("index/").into(),
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
    // The prover needs some way to pull blocks from a trusted source, we can use anything
    // implementing the [Blockchain] trait, for example a bitcoin core node or an esplora
    // instance.
    let client = get_chain_provider()?;
    // Create a prover, this module will download blocks from the bitcoin core
    // node and save them to disk. It will also create proofs for the blocks
    // and save them to disk.
    let mut prover = prover::Prover::new(client, index_store.clone(), blocks.clone(), view.clone());
    info!("Starting p2p node");
    // This is our implementation of the Bitcoin p2p protocol, it will listen
    // for incoming connections and serve blocks and proofs to peers.
    let p2p_port = env::var("P2P_PORT").unwrap_or_else(|_| "8333".into());
    let p2p_address = format!(
        "{}:{}",
        env::var("P2P_HOST").unwrap_or_else(|_| "0.0.0.0".into()),
        p2p_port
    );
    let listener = std::net::TcpListener::bind(p2p_address).unwrap();
    let node = node::Node::new(listener, blocks, index_store, view);
    std::thread::spawn(move || {
        Node::accept_connections(node);
    });
    let (sender, receiver) = channel(1024);
    // This is our implementation of the json-rpc api, it will listen for
    // incoming connections and serve some Utreexo data to clients.
    info!("Starting api");
    std::thread::spawn(|| {
        actix_rt::System::new()
            .block_on(api::create_api(sender))
            .unwrap()
    });

    let kill_signal = Arc::new(Mutex::new(false));
    let kill_signal2 = kill_signal.clone();

    // Keep the prover running in the background, it will download blocks and
    // create proofs for them as they are mined.
    info!("Running prover");
    std::thread::spawn(move || {
        actix_rt::System::new().block_on(async {
            let _ = ctrl_c().await;
            warn!("Received a stop signal");
            *kill_signal.lock().unwrap() = true;
        })
    });

    prover.keep_up(kill_signal2, receiver)
}

fn subdir(path: &str) -> String {
    let dir = env::var("DATA_DIR").unwrap_or_else(|_| {
        let dir = env::var("HOME").expect("No $HOME env var?");
        dir + "/.bridge"
    });
    dir + "/" + path
}

fn init_logger(log_file: Option<&str>, log_level: log::LevelFilter, log_to_term: bool) {
    let mut loggers: Vec<Box<dyn SharedLogger>> = vec![];
    if let Some(file) = log_file {
        let file_logger = simplelog::WriteLogger::new(
            log_level,
            Config::default(),
            std::fs::File::create(file).unwrap(),
        );
        loggers.push(file_logger);
    }
    if log_to_term {
        let term_logger = simplelog::TermLogger::new(
            log_level,
            Config::default(),
            simplelog::TerminalMode::Mixed,
            simplelog::ColorChoice::Auto,
        );
        loggers.push(term_logger);
    }
    if loggers.is_empty() {
        eprintln!("No logger specified, logging disabled");
        return;
    }
    let _ = simplelog::CombinedLogger::init(loggers);
}

fn get_chain_provider() -> Result<Box<dyn Blockchain>> {
    #[cfg(feature = "esplora")]
    if let Ok(esplora_url) = env::var("ESPLORA_URL") {
        return Ok(Box::new(esplora::EsploraBlockchain::new(esplora_url)));
    }
    let rpc_url = env::var("BITCOIN_CORE_RPC_URL").unwrap_or_else(|_| "localhost:8332".into());
    // try to use username and password auth first
    if let Ok(username) = env::var("BITCOIN_CORE_RPC_USER") {
        let password = env::var("BITCOIN_CORE_RPC_PASSWORD").map_err(|_| {
            anyhow::anyhow!("BITCOIN_CORE_RPC_PASSWORD must be set if BITCOIN_CORE_RPC_USER is set")
        })?;
        info!(
            "Using bitcoin core at {} with username {}",
            rpc_url, username
        );
        let client = Client::new(&rpc_url, Auth::UserPass(username, password));
        match client {
            Ok(client) => {
                return Ok(Box::new(client));
            }
            Err(e) => return Err(anyhow::anyhow!("Couldn't connect to bitcoin core: {e}")),
        }
    }
    // fallback to cookie auth. This is the default for core, but discouraged for security reasons
    let cookie = env::var("BITCOIN_CORE_COOKIE_FILE").unwrap_or_else(|_| {
        env::var("HOME")
            .map(|home| format!("{}/.bitcoin/.cookie", home))
            .expect("Failed to find $HOME")
    });
    info!("Using cookie file at {}", cookie);
    let client = Client::new(&rpc_url, Auth::CookieFile(cookie.clone().into()));
    match client {
        Ok(client) => {
            info!("Using bitcoin core at {}", rpc_url);
            return Ok(Box::new(client));
        }
        Err(e) => Err(anyhow::anyhow!("Couldn't connect to bitcoin core: {e}")),
    }
}
