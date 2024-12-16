//SPDX-License-Identifier: MIT

/// A simple and efficient implementation of Bitcoin P2P network
///
/// This bridge software gives you the option to serve proofs and blocks over the
/// p2p port, just like any other node. The way it works is simple and meant to use
/// as little deps as possible. We explicitly don't use async/await on this.
///
/// ## Architecture
///
/// This implementation uses async IO, but without the overhead of proper async/await.
/// It is broken down in three modules: [Reactor], [Worker] and [Peer].
///   - [Reactor]: We only have one of those, it will run in a loop, polling different sockets for events. If
///                the socket gets ready for read/write, the reactor should send the socket
///                and ID to one of our workers. The worker selection is random and assumes
///                that they will take about the same time to handle each socket. In the future
///                we may need to add a proper work distribution mechanism.
///
///   - [Worker]: We have a couple of workers, and each worker has one OS thread. When a socket is
///               ready, one [Worker] will receive it using a channel. The actual [Peer] state is
///               kept inside our workers, inside a shared vector. So, after receiver a notification,
///               we pick the corresponding [Peer] and call `handle_request` to read from the socket
///               and handle the request. `handle_request` don't write to the socket, only to a
///               buffer. If needed, the worker will also write back that data.
///
///   - [Peer]: Holds all the context related to a [Peer] and handles requests. Since we may not
///             read a hole [NetworkMessage] at once (or read more than one), we buffer everything
///             in a read buffer. We also have a write buffer, and this is to both avoid too many
///             syscalls, but also to avoid calling write too often, which should cause the socket
///             to err on `WouldBlock`. If we don't succeed in sending all the buffer, the [Worker]
///             will schedule us for write events (we won't read before finishing the write).
use std::cell::UnsafeCell;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt::Display;
use std::io::Read;
use std::io::Write;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::sync::PoisonError;
use std::sync::RwLock;
use std::time::Duration;
use std::time::Instant;

use bitcoin::consensus::deserialize;
use bitcoin::consensus::serialize;
use bitcoin::consensus::Decodable;
use bitcoin::hashes::Hash;
use bitcoin::key::rand::random;
use bitcoin::p2p::message::NetworkMessage;
use bitcoin::p2p::message::RawNetworkMessage;
use bitcoin::p2p::message_blockdata::Inventory;
use bitcoin::p2p::message_filter::CFilter;
use bitcoin::p2p::message_network::VersionMessage;
use bitcoin::p2p::Magic;
use bitcoin::p2p::ServiceFlags;
use bitcoin::BlockHash;
use log::debug;
use log::error;
use log::info;
use mio::net::TcpListener;
use mio::net::TcpStream;
use sha2::Digest;
use sha2::Sha256;

use crate::block_index::BlocksIndex;
use crate::blockfile::BlockFile;
use crate::chainview::ChainView;

const FILTER_TYPE_UTREEXO: u8 = 1;
const WORKES_PER_CLUSTER: usize = 4;

#[derive(Debug)]
/// A minimal version of the message header
///
/// We need this because `rust-bitcoin` won't let us read only the reader, only the full message.
/// But we may need this to figure out whether we have all the data, or should just wait for a
/// while.
pub struct P2PMessageHeader {
    /// Magic data that's always in the beginning of a message
    ///
    /// This constant is defined per-network, and if it doesn't match what we expected, we'll
    /// disconnect with that peer
    _magic: Magic,
    /// A command string telling what this message should be (e.g.: block, inv, tx)
    _command: [u8; 12],
    /// How long this message is
    length: u32,
    /// The payload's checksum
    _checksum: u32,
}

#[derive(Clone)]
/// Data required by our peers to handle requests
pub struct WorkerContext {
    /// The actual blocks and proofs
    pub proof_backend: Arc<RwLock<BlockFile>>,
    /// An index on [BlockFile] so we can get specific blocks
    pub proof_index: Arc<BlocksIndex>,
    /// Our chain metadata. Things like our height, and  an index height -> hash
    pub chainview: Arc<ChainView>,
    /// The magic bits for the network we are on
    pub magic: Magic,
}

impl Decodable for P2PMessageHeader {
    fn consensus_decode<R: bitcoin::io::Read + ?Sized>(
        reader: &mut R,
    ) -> std::result::Result<Self, bitcoin::consensus::encode::Error> {
        let _magic = Magic::consensus_decode(reader)?;
        let _command = <[u8; 12]>::consensus_decode(reader)?;
        let length = u32::consensus_decode(reader)?;
        let _checksum = u32::consensus_decode(reader)?;
        Ok(Self {
            _checksum,
            _command,
            length,
            _magic,
        })
    }
}

/// A struct that will set everything up and running. It doesn't have any state nor runs on any
/// thread, but after calling `run` you should have the [Reactor] and [Worker]s running
pub struct Node;

/// Local context for each peer
///
/// This will perform all the processing and IO related to a given peer, it can will be owned by
/// our workers and used every time the reactor tell us there's something available
pub struct Peer {
    /// Data that we've read, but didn't process
    read_buffer: Vec<u8>,
    /// Data that we need to write into the socket
    write_buffer: Vec<u8>,
    /// Where we can get blocks to send to peers
    proof_backend: Arc<RwLock<BlockFile>>,
    /// Index to learn where things are inside the [BlockFile]
    proof_index: Arc<BlocksIndex>,
    /// General info about our chain
    chainview: Arc<ChainView>,
    /// Magic bits used in every network message
    magic: Magic,
}

#[derive(Debug)]
/// Errors returned by the [Peer] when processing requests
enum PeerError {
    /// Io Error
    Io(std::io::Error),
    /// Can't decode the message
    Decode(bitcoin::consensus::encode::Error),
    /// Some lock is poisoned (a thread died while holding it)
    Poison,
    /// The provided magic value is invalid
    InvalidMagic,
    /// The message we got is too big
    MessageTooLarge,
}

impl Display for PeerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PeerError::Io(e) => write!(f, "IO error: {}", e),
            PeerError::Decode(e) => write!(f, "Decode error: {}", e),
            PeerError::Poison => write!(f, "Lock is poisoned"),
            PeerError::InvalidMagic => write!(f, "Invalid magic"),
            PeerError::MessageTooLarge => write!(f, "Message too large"),
        }
    }
}

impl From<std::io::Error> for PeerError {
    fn from(e: std::io::Error) -> Self {
        PeerError::Io(e)
    }
}

impl From<bitcoin::consensus::encode::Error> for PeerError {
    fn from(e: bitcoin::consensus::encode::Error) -> Self {
        PeerError::Decode(e)
    }
}

impl<T> From<PoisonError<T>> for PeerError {
    fn from(_: PoisonError<T>) -> Self {
        PeerError::Poison
    }
}

impl Node {
    /// Spawn a reactor and some workers to handle requests.
    ///
    /// If we can't set things up, this function will panic
    pub fn run(
        address: SocketAddr,
        worker_context: WorkerContext,
        block_notifier: Receiver<BlockHash>,
    ) {
        let listener = TcpListener::bind(address).expect("Failed to bind to address");
        let (register, register_rx) = std::sync::mpsc::channel();
        let reactor = Reactor {
            block_notifier,
            listener,
            registered: HashMap::new(),
            register: register_rx,
            worker_pool: Self::create_workers(&worker_context, register),
            magic: worker_context.magic,
            pings: BTreeMap::new(),
            timeouts: BTreeMap::new(),
        };

        std::thread::Builder::new()
            .name("bridge - reactor thread".to_string())
            .spawn(move || reactor.run())
            .expect("Failed to spawn reactor");
    }

    /// spawns our workers
    fn create_workers(
        worker_context: &WorkerContext,
        scheduler: Sender<(usize, TcpStream, Intent)>,
    ) -> [Sender<Message>; WORKES_PER_CLUSTER] {
        let mut workers = Vec::new();
        let peers = Rc::new(UnsafeCell::new(HashMap::new()));
        for i in 0..WORKES_PER_CLUSTER {
            let (tx, rx) = std::sync::mpsc::channel();
            let worker = Worker::new(
                i,
                peers.clone(),
                worker_context.proof_backend.clone(),
                worker_context.proof_index.clone(),
                worker_context.chainview.clone(),
                worker_context.magic,
                rx,
                scheduler.clone(),
            );

            std::thread::Builder::new()
                .name(format!("bridge - worker {}", i))
                .spawn(move || worker.run())
                .expect("Failed to spawn worker");

            workers.push(tx);
        }

        workers.try_into().expect("Failed to create workers")
    }
}

#[derive(Debug)]
/// Messages sent from [Reactor] to [Worker]
pub enum Message {
    /// The server got a new connection
    ///
    /// Once a worker gets this, it'll construct a [Peer] struct and schedule it for the next
    /// message
    NewConnection(TcpStream),
    /// There's something to read in the socket
    ReadReady((TcpStream, usize)),
    /// We can write to the socket
    WriteReady((TcpStream, usize)),
    /// Some peer disconnected (either them or us killed the socket)
    Disconnect(usize),
}

#[derive(Debug)]
/// What are we waiting for
enum Intent {
    Read,
    Write,
}

/// A struct that will run and wait for work to do. Once it gets a new work from [Reactor], it will
/// call the relevant functions and use its cpu share to make progress
struct Worker {
    /// A unique per-worker identifier
    id: usize,
    /// The channel used by [Reactor] to notify us about new things to do
    job_receiver: Receiver<Message>,
    /// A shared memory region that holds all our [Peer]s
    ///
    /// Since we know that never two workers will try to operate on the same [Peer], and also that
    /// all changes to the actual [HashMap] will always be made by worker `0` (see the reactor
    /// bellow). We don't need to worry about synchronization here. Moreover, we'll never drop
    /// this, as our workers lives through the entire lifetime of our program, we don't need an
    /// [Arc].
    peers: Rc<UnsafeCell<HashMap<usize, Peer>>>,
    /// A channel to ask the [Reactor] to notify us about new events
    scheduler: Sender<(usize, TcpStream, Intent)>,

    /// We use those to build [Peer]
    proof_backend: Arc<RwLock<BlockFile>>,
    proof_index: Arc<BlocksIndex>,
    chainview: Arc<ChainView>,
    magic: Magic,
}

unsafe impl Sync for Worker {}
unsafe impl Send for Worker {}

impl Worker {
    /// Creates a new worker
    ///
    /// This function doesn't spawn any thread, the caller is responsible for running [Worker::run]
    /// inside a thread
    fn new(
        id: usize,
        peers: Rc<UnsafeCell<HashMap<usize, Peer>>>,
        proof_backend: Arc<RwLock<BlockFile>>,
        proof_index: Arc<BlocksIndex>,
        chainview: Arc<ChainView>,
        magic: Magic,
        job_receiver: Receiver<Message>,
        scheduler: Sender<(usize, TcpStream, Intent)>,
    ) -> Self {
        Self {
            peers,
            scheduler,
            id,
            proof_backend,
            proof_index,
            chainview,
            magic,
            job_receiver,
        }
    }

    /// Takes ownership of the worker and runs until this thread dies
    fn run(self) {
        debug!("Worker {} started", self.id);
        loop {
            let job = self
                .job_receiver
                .recv()
                .expect("job_receiver channel is broken");

            match job {
                Message::NewConnection(socket) => {
                    let peer = Peer::new(
                        "unknown".to_string(),
                        "unknown".to_string(),
                        self.proof_backend.clone(),
                        self.proof_index.clone(),
                        self.chainview.clone(),
                        self.magic,
                    );

                    let id: usize = random();
                    let peers = unsafe { &mut *self.peers.get() };

                    peers.insert(id, peer);
                    self.scheduler
                        .send((id, socket, Intent::Read))
                        .expect("reactor died");
                }

                Message::ReadReady((mut stream, id)) => {
                    debug!("worker: got read event for peer {id}");

                    let peers = unsafe { &mut *self.peers.get() };
                    let Some(peer) = peers.get_mut(&id) else {
                        log::error!("can't find peer: {id}");
                        peers.remove(&id);
                        continue;
                    };

                    if let Err(err) = peer.handle_request(&mut stream) {
                        log::error!("Error handling request: {}", err);
                        if let PeerError::Io(ref e) = err {
                            if e.kind() == std::io::ErrorKind::WouldBlock {
                                self.scheduler
                                    .send((id, stream, Intent::Read))
                                    .expect("reactor died");

                                continue;
                            }
                        }

                        peers.remove(&id);
                        continue;
                    }

                    match peer.write_back(&mut stream) {
                        Ok(true) => self
                            .scheduler
                            .send((id, stream, Intent::Read))
                            .expect("reactor died"),

                        Ok(false) => self
                            .scheduler
                            .send((id, stream, Intent::Write))
                            .expect("reactor died"),

                        Err(err) => {
                            log::error!("Error handling request: {}", err);
                            if let PeerError::Io(ref e) = err {
                                if e.kind() == std::io::ErrorKind::WouldBlock {
                                    self.scheduler
                                        .send((id, stream, Intent::Write))
                                        .expect("reactor died");
                                    continue;
                                }
                            }

                            peers.remove(&id);
                            continue;
                        }
                    }
                }

                Message::WriteReady((mut stream, id)) => {
                    debug!("worker: got event for peer {id}");

                    let peers = unsafe { &mut *self.peers.get() };
                    let Some(peer) = peers.get_mut(&id) else {
                        log::error!("can't find peer: {id}");
                        peers.remove(&id);
                        continue;
                    };

                    match peer.write_back(&mut stream) {
                        Ok(true) => self
                            .scheduler
                            .send((id, stream, Intent::Read))
                            .expect("reactor died"),

                        Ok(false) => self
                            .scheduler
                            .send((id, stream, Intent::Write))
                            .expect("reactor died"),

                        Err(err) => {
                            log::error!("Error handling request: {}", err);
                            if let PeerError::Io(ref e) = err {
                                if e.kind() == std::io::ErrorKind::WouldBlock {
                                    self.scheduler
                                        .send((id, stream, Intent::Write))
                                        .expect("reactor died");
                                    continue;
                                }
                            }

                            peers.remove(&id);
                            continue;
                        }
                    }
                }

                Message::Disconnect(id) => {
                    info!("worker: peer {id} disconnected");

                    let peers = unsafe { &mut *self.peers.get() };
                    peers.remove(&id);
                }
            }
        }
    }
}

/// Keep watching sockets for new events
struct Reactor {
    /// Our server's listener
    ///
    /// Used to accept new p2p connections
    listener: TcpListener,
    /// Channels to our workers
    worker_pool: [Sender<Message>; WORKES_PER_CLUSTER],
    /// A channel we use to receive things to watch
    register: Receiver<(usize, TcpStream, Intent)>,
    /// Sockets we are watching
    registered: HashMap<usize, (TcpStream, Intent)>,
    /// This channel will notify us about new blocks
    block_notifier: Receiver<BlockHash>,
    /// Magic value for this network
    magic: Magic,
    /// If a peer doesn't send us a message for too long, poke it to see if it's still alive
    timeouts: BTreeMap<Instant, usize>,
    /// pings we've sent
    pings: BTreeMap<Instant, usize>,
}

impl Reactor {
    fn run(mut self) {
        let mut poll = mio::Poll::new().expect("can't create a mio Poller");
        poll.registry()
            .register(&mut self.listener, mio::Token(0), mio::Interest::READABLE)
            .expect("Failed to register listener");

        let mut events = mio::Events::with_capacity(1024);
        loop {
            let registry = poll.registry();
            while let Ok(mut work) = self.register.try_recv() {
                debug!("reactor: registering {} for {:?}", work.0, work.2);

                self.timeouts
                    .insert(Instant::now() + Duration::from_secs(10 * 60), work.0);

                let intent = match work.2 {
                    Intent::Read => mio::Interest::READABLE,
                    Intent::Write => mio::Interest::WRITABLE,
                };

                match registry.register(&mut work.1, mio::Token(work.0), intent) {
                    Ok(_) => {}
                    Err(e) => {
                        log::error!("Failed to register socket: {}", e);
                        continue;
                    }
                }

                self.registered.insert(work.0, (work.1, work.2));
            }

            let next_timeout = self
                .timeouts
                .iter()
                .next()
                .map(|(t, _)| t.duration_since(Instant::now()));

            if let Err(e) = poll.poll(
                &mut events,
                Some(next_timeout.unwrap_or(Duration::from_secs(1))),
            ) {
                log::error!("Failed to poll: {}", e);
                continue;
            }

            self.block_notifier.try_iter().for_each(|block| {
                let inv = RawNetworkMessage::new(
                    self.magic,
                    NetworkMessage::Inv(vec![Inventory::Block(block)]),
                );

                let msg = serialize(&inv);

                self.registered.iter_mut().for_each(|(_, (socket, _))| {
                    let _ = socket.write_all(&msg);
                });
            });

            let registry = poll.registry();
            for event in events.iter() {
                match event.token() {
                    mio::Token(0) => {
                        debug!("reactor: our listener got a new event");

                        let Ok((stream, address)) = self.listener.accept() else {
                            log::error!("Failed to accept connection");
                            continue;
                        };

                        info!("reactor: saw new connection from {address}");

                        self.worker_pool[0]
                            .send(Message::NewConnection(stream))
                            .expect("Failed to send new connection to worker");
                    }

                    mio::Token(token) => {
                        debug!("reactor: event for {token}");

                        if event.is_read_closed() || event.is_write_closed() || event.is_error() {
                            let (mut socket, _) = self
                                .registered
                                .remove(&token)
                                .expect("BUG: socket is registered but not in registered map");

                            if let Err(e) = registry.deregister(&mut socket) {
                                error!("can't deregister socket {token} due to {e:?}");
                                continue;
                            }

                            self.pings.retain(|_, id| id != &token);
                            self.timeouts.retain(|_, id| id != &token);
                            self.worker_pool[0]
                                .send(Message::Disconnect(token))
                                .expect("Failed to send disconnect to worker");
                            continue;
                        }

                        let worker_id = random::<usize>() % self.worker_pool.len();
                        let worker = self.worker_pool.get(worker_id).expect("broken worker");
                        let (mut socket, intent) = self
                            .registered
                            .remove(&token)
                            .expect("BUG: socket is registered but not in registered map");

                        if let Err(e) = registry.deregister(&mut socket) {
                            error!("can't deregister socket {token} due to {e:?}");
                            continue;
                        }

                        self.pings.retain(|_, id| id != &token);
                        self.timeouts.retain(|_, id| id != &token);

                        debug!("reactor: sending job to worker {}", worker_id);
                        match intent {
                            Intent::Read => {
                                worker
                                    .send(Message::ReadReady((socket, token)))
                                    .expect("Failed to send to worker");
                            }
                            Intent::Write => {
                                worker
                                    .send(Message::WriteReady((socket, token)))
                                    .expect("Failed to send to worker");
                            }
                        }
                    }
                }
            }

            let now = Instant::now();
            let new_timeout = self.timeouts.split_off(&now);

            for (_, id) in self.timeouts {
                debug!("reactor: sending ping to {id}");
                let nonce = random();
                let ping = RawNetworkMessage::new(self.magic, NetworkMessage::Ping(nonce));

                let msg = serialize(&ping);
                let (socket, _) = self
                    .registered
                    .get_mut(&id)
                    .expect("BUG: socket is registered but not in registered map");

                match socket.write_all(&msg) {
                    Ok(_) => {
                        self.pings.insert(now + Duration::from_secs(30), id);
                    }
                    Err(e) => {
                        log::error!("Failed to send ping: {}", e);
                        self.registered.remove(&id);
                        self.worker_pool[0]
                            .send(Message::Disconnect(id))
                            .expect("Failed to send disconnect to worker");
                        continue;
                    }
                }
            }

            self.timeouts = new_timeout;
            for (time, id) in self.pings.iter() {
                // request timed out, assume peer died
                if *time < now {
                    debug!("reactor: peer {id} timed out ping");

                    self.worker_pool[0]
                        .send(Message::Disconnect(*id))
                        .expect("Failed to send disconnect to worker");
                    self.registered.remove(id);
                }
            }
        }
    }
}

impl Peer {
    pub fn new(
        _peer: String,
        _peer_id: String,
        proof_backend: Arc<RwLock<BlockFile>>,
        proof_index: Arc<BlocksIndex>,
        chainview: Arc<ChainView>,
        magic: Magic,
    ) -> Self {
        Self {
            proof_backend,
            proof_index,
            chainview,
            magic,
            write_buffer: Vec::new(),
            read_buffer: Vec::new(),
        }
    }

    fn consume_message(&mut self) -> Result<Option<RawNetworkMessage>, PeerError> {
        let mut reader = self.read_buffer.as_slice();
        let header = P2PMessageHeader::consensus_decode(&mut reader)?;
        if header.length > 32_000_000 {
            return Err(PeerError::MessageTooLarge);
        }

        if header._magic != self.magic {
            return Err(PeerError::InvalidMagic);
        }

        if self.read_buffer.len() < (header.length + 24) as usize {
            return Ok(None);
        }

        let data = self.read_buffer.drain(0..(24 + header.length as usize));
        let message = RawNetworkMessage::consensus_decode(&mut data.as_slice())?;
        Ok(Some(message))
    }

    fn send_message(&mut self, message: NetworkMessage) -> Result<(), PeerError> {
        let msg = RawNetworkMessage::new(self.magic, message);
        self.write_buffer.extend(&serialize(&msg));

        Ok(())
    }

    fn read_pending(&mut self, stream: &mut TcpStream) -> Result<usize, PeerError> {
        let mut buffer = vec![0; 32_000_000];
        let read = stream.read(&mut buffer)?;

        self.read_buffer.extend(buffer.drain(0..read));
        Ok(read)
    }

    fn sha256d_payload(&self, payload: &[u8]) -> [u8; 32] {
        let mut sha = Sha256::new();
        sha.update(payload);

        let hash = sha.finalize();
        let mut sha = sha2::Sha256::new();
        sha.update(hash);

        sha.finalize().into()
    }

    fn write_back(&mut self, stream: &mut TcpStream) -> Result<bool, PeerError> {
        debug!("peer: writing back {} bytes", self.write_buffer.len());

        let writen = stream.write(&self.write_buffer)?;
        self.write_buffer.drain(0..writen);
        Ok(self.write_buffer.is_empty())
    }

    fn handle_request(&mut self, stream: &mut TcpStream) -> Result<(), PeerError> {
        let read = self.read_pending(stream)?;
        debug!("peer: read {read} bytes");

        loop {
            if self.read_buffer.len() < 24 {
                break;
            }

            let Some(request) = self.consume_message()? else {
                break;
            };

            self.handle_request_inner(request, stream)?;
        }

        Ok(())
    }

    fn handle_request_inner(
        &mut self,
        request: RawNetworkMessage,
        stream: &mut TcpStream,
    ) -> Result<(), PeerError> {
        match request.payload() {
            NetworkMessage::Ping(nonce) => {
                let pong = NetworkMessage::Pong(*nonce);
                self.send_message(pong)?;
            }

            NetworkMessage::GetData(inv) => {
                let mut blocks = vec![];
                for el in inv {
                    match el {
                        Inventory::Unknown { hash, inv_type } => {
                            if *inv_type != 0x41000002 {
                                continue;
                            }
                            let block_hash = BlockHash::from_byte_array(*hash);
                            let Some(block) = self.proof_index.get_index(block_hash) else {
                                let not_found =
                                    NetworkMessage::NotFound(vec![Inventory::Unknown {
                                        inv_type: 0x41000002,
                                        hash: *hash,
                                    }]);

                                self.send_message(not_found)?;
                                continue;
                            };

                            let lock = self.proof_backend.read()?;
                            let payload = lock.get_block_slice(block);
                            let checksum = &self.sha256d_payload(payload)[0..4];

                            let mut message_header = [0u8; 24];
                            message_header[0..4].copy_from_slice(&request.magic().to_bytes());
                            message_header[4..9].copy_from_slice("block".as_bytes());
                            message_header[16..20]
                                .copy_from_slice(&(payload.len() as u32).to_le_bytes());
                            message_header[20..24].copy_from_slice(checksum);

                            stream.write_all(&message_header)?;
                            stream.write_all(payload)?;
                        }
                        Inventory::WitnessBlock(block_hash) => {
                            let Some(block) = self.proof_index.get_index(*block_hash) else {
                                let not_found =
                                    NetworkMessage::NotFound(vec![Inventory::WitnessBlock(
                                        *block_hash,
                                    )]);
                                self.send_message(not_found)?;
                                continue;
                            };
                            let lock = self.proof_backend.read().expect("lock failed");
                            match lock.get_block(block) {
                                //TODO: Rust-Bitcoin asks for a block, but we have it serialized on disk already.
                                //      We should be able to just send the block without deserializing it.
                                Some(block) => {
                                    let block = NetworkMessage::Block(block.into());
                                    blocks.push(block);
                                }
                                None => {
                                    let not_foud =
                                        NetworkMessage::NotFound(vec![Inventory::WitnessBlock(
                                            *block_hash,
                                        )]);

                                    let res = not_foud;
                                    blocks.push(res);
                                }
                            }
                        }
                        // TODO: Prove mempool txs
                        _ => {}
                    }
                }

                for block in blocks {
                    self.send_message(block)?;
                }
            }

            NetworkMessage::GetHeaders(locator) => {
                let mut headers = vec![];
                let Some(block) = locator.locator_hashes.first() else {
                    return Ok(());
                };

                let height = self.chainview.get_height(*block).unwrap().unwrap_or(0);

                let height = height + 1;
                for h in height..(height + 2_000) {
                    let Ok(Some(block_hash)) = self.chainview.get_block_hash(h) else {
                        break;
                    };

                    let Ok(Some(header_info)) = self.chainview.get_block(block_hash) else {
                        break;
                    };

                    let header = deserialize(&header_info)?;
                    headers.push(header);
                }

                let headers = NetworkMessage::Headers(headers);
                self.send_message(headers)?;
            }

            NetworkMessage::Version(version) => {
                info!(
                    "Handshake success version={} blocks={} services={} address={:?} address_our={:?}",
                    version.user_agent,
                    version.start_height,
                    version.services,
                    version.receiver.address,
                    version.sender.address
                );

                let version = NetworkMessage::Version(VersionMessage {
                    version: 70001,
                    services: ServiceFlags::NETWORK_LIMITED
                                | ServiceFlags::NETWORK
                                | ServiceFlags::WITNESS
                                | ServiceFlags::from(1 << 24)  // UTREEXO
                                | ServiceFlags::from(1 << 25), // UTREEXO_BLOCK_FILTERS
                    timestamp: version.timestamp + 1,
                    receiver: version.sender.clone(),
                    sender: version.receiver.clone(),
                    nonce: version.nonce + 100,
                    user_agent: "/rustreexo:0.1.0/bridge:0.1.0".to_string(),
                    start_height: self.proof_index.load_height() as i32,
                    relay: false,
                });

                let our_version = version;
                self.send_message(our_version)?;

                let verack = NetworkMessage::Verack;
                self.send_message(verack)?;
            }

            NetworkMessage::GetCFilters(req) => {
                if req.filter_type == FILTER_TYPE_UTREEXO {
                    let Ok(Some(acc)) = self.chainview.get_acc(req.stop_hash) else {
                        // if this block is not in the chainview, ignore the request
                        return Ok(());
                    };

                    let cfilters = NetworkMessage::CFilter(CFilter {
                        filter_type: FILTER_TYPE_UTREEXO,
                        block_hash: req.stop_hash,
                        filter: acc,
                    });

                    self.send_message(cfilters)?;
                }

                // ignore unknown filter types
            }

            _ => {}
        }

        Ok(())
    }
}
