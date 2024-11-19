//SPDX-License-Identifier: MIT

use std::fmt::Display;
use std::io::Read;
use std::io::Write;
use std::net::TcpListener;
use std::net::TcpStream;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::sync::RwLock;

use bitcoin::consensus::deserialize;
use bitcoin::consensus::serialize;
use bitcoin::consensus::Decodable;
use bitcoin::consensus::Encodable;
use bitcoin::hashes::Hash;
use bitcoin::p2p::message::NetworkMessage;
use bitcoin::p2p::message::RawNetworkMessage;
use bitcoin::p2p::message_blockdata::Inventory;
use bitcoin::p2p::message_filter::CFilter;
use bitcoin::p2p::message_network::VersionMessage;
use bitcoin::p2p::Magic;
use bitcoin::p2p::ServiceFlags;
use bitcoin::BlockHash;
use log::error;
use log::info;
use sha2::Digest;
use sha2::Sha256;

use crate::block_index::BlocksIndex;
use crate::blockfile::BlockFile;
use crate::chainview::ChainView;
use crate::try_and_log_error;

const FILTER_TYPE_UTREEXO: u8 = 1;

pub struct P2PMessageHeader {
    _magic: Magic,
    _command: [u8; 12],
    length: u32,
    _checksum: u32,
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
pub struct Node {
    listener: TcpListener,
    proof_backend: Arc<RwLock<BlockFile>>,
    proof_index: Arc<BlocksIndex>,
    chainview: Arc<ChainView>,
    sockets: Arc<RwLock<Vec<TcpStream>>>,
}

pub struct Peer {
    proof_backend: Arc<RwLock<BlockFile>>,
    reader: TcpStream,
    writer: TcpStream,
    proof_index: Arc<BlocksIndex>,
    chainview: Arc<ChainView>,
}

#[derive(Debug)]
enum ReadError {
    IoError(std::io::Error),
    DecodeError(bitcoin::consensus::encode::Error),
}

impl Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::IoError(e) => write!(f, "IO error: {}", e),
            ReadError::DecodeError(e) => write!(f, "Decode error: {}", e),
        }
    }
}

impl From<std::io::Error> for ReadError {
    fn from(e: std::io::Error) -> Self {
        Self::IoError(e)
    }
}

impl From<bitcoin::consensus::encode::Error> for ReadError {
    fn from(e: bitcoin::consensus::encode::Error) -> Self {
        Self::DecodeError(e)
    }
}

impl Peer {
    pub fn new(
        stream: TcpStream,
        _peer: String,
        _peer_id: String,
        proof_backend: Arc<RwLock<BlockFile>>,
        proof_index: Arc<BlocksIndex>,
        chainview: Arc<ChainView>,
    ) -> Self {
        let reader = stream.try_clone().unwrap();
        Self {
            proof_backend,
            proof_index,
            reader,
            writer: stream,
            chainview,
        }
    }

    fn send_message(&mut self, message: RawNetworkMessage) {
        let mut msg = Vec::new();
        message.consensus_encode(&mut msg).unwrap();

        try_and_log_error!(self.writer.write_all(&msg));
    }

    fn read_header(&mut self) -> Result<([u8; 24], P2PMessageHeader), ReadError> {
        let mut raw_header: [u8; 24] = [0; 24];
        self.reader.read_exact(&mut raw_header)?;

        let mut reader = raw_header.as_slice();
        Ok((raw_header, P2PMessageHeader::consensus_decode(&mut reader)?))
    }

    fn sha256d_payload(&self, payload: &[u8]) -> [u8; 32] {
        let mut sha = Sha256::new();
        sha.update(payload);

        let hash = sha.finalize();
        let mut sha = sha2::Sha256::new();
        sha.update(hash);

        sha.finalize().into()
    }

    fn handle_request(&mut self) -> Result<(), ReadError> {
        let (raw_header, parsed_header) = self.read_header()?;

        if parsed_header.length > 32_000_000 {
            return Ok(());
        }

        let mut raw_payload = vec![0; (parsed_header.length + 24) as usize];
        raw_payload[..24].copy_from_slice(&raw_header);
        self.reader.read_exact(&mut raw_payload[24..])?;

        let mut reader = raw_payload.as_slice();
        let request = RawNetworkMessage::consensus_decode(&mut reader)?;

        match request.payload() {
            NetworkMessage::Ping(nonce) => {
                let pong = RawNetworkMessage::new(*request.magic(), NetworkMessage::Pong(*nonce));

                self.send_message(pong);
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

                                let res = RawNetworkMessage::new(*request.magic(), not_found);
                                self.send_message(res);
                                continue;
                            };

                            let lock = self.proof_backend.read().unwrap();
                            let payload = lock.get_block_slice(block);
                            let checksum = &self.sha256d_payload(payload)[0..4];

                            let mut message_header = [0u8; 24];
                            message_header[0..4].copy_from_slice(&request.magic().to_bytes());
                            message_header[4..9].copy_from_slice("block".as_bytes());
                            message_header[16..20]
                                .copy_from_slice(&(payload.len() as u32).to_le_bytes());
                            message_header[20..24].copy_from_slice(checksum);

                            self.writer.write_all(&message_header)?;
                            self.writer.write_all(payload)?;
                        }
                        Inventory::WitnessBlock(block_hash) => {
                            let Some(block) = self.proof_index.get_index(*block_hash) else {
                                let res = RawNetworkMessage::new(
                                    *request.magic(),
                                    NetworkMessage::NotFound(vec![Inventory::WitnessBlock(
                                        *block_hash,
                                    )]),
                                );
                                self.send_message(res);
                                continue;
                            };
                            let lock = self.proof_backend.read().unwrap();
                            match lock.get_block(block) {
                                //TODO: Rust-Bitcoin asks for a block, but we have it serialized on disk already.
                                //      We should be able to just send the block without deserializing it.
                                Some(block) => {
                                    let block = RawNetworkMessage::new(
                                        *request.magic(),
                                        NetworkMessage::Block(block.into()),
                                    );

                                    blocks.push(block);
                                }
                                None => {
                                    let not_foud =
                                        NetworkMessage::NotFound(vec![Inventory::WitnessBlock(
                                            *block_hash,
                                        )]);

                                    let res = RawNetworkMessage::new(*request.magic(), not_foud);
                                    blocks.push(res);
                                }
                            }
                        }
                        // TODO: Prove mempool txs
                        _ => {}
                    }
                }

                blocks
                    .into_iter()
                    .for_each(|block| self.send_message(block));
            }

            NetworkMessage::GetHeaders(locator) => {
                let mut headers = vec![];
                let block = *locator.locator_hashes.first().unwrap();
                let height = self.chainview.get_height(block).unwrap().unwrap_or(0);
                let height = height + 1;

                for h in height..(height + 2_000) {
                    let Ok(Some(block_hash)) = self.chainview.get_block_hash(h) else {
                        break;
                    };

                    let Ok(Some(header_info)) = self.chainview.get_block(block_hash) else {
                        break;
                    };

                    let header = deserialize(&header_info).unwrap();
                    headers.push(header);
                }

                let headers =
                    RawNetworkMessage::new(*request.magic(), NetworkMessage::Headers(headers));

                self.send_message(headers);
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

                let our_version = RawNetworkMessage::new(*request.magic(), version);
                self.send_message(our_version);

                let verack = RawNetworkMessage::new(*request.magic(), NetworkMessage::Verack);
                self.send_message(verack);
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

                    let cfilter = RawNetworkMessage::new(*request.magic(), cfilters);
                    self.send_message(cfilter);
                }

                // ignore unknown filter types
            }

            _ => {}
        }
        Ok(())
    }

    pub fn peer_loop(mut self) {
        loop {
            if self.handle_request().is_err() {
                info!("Connection closed");
                break;
            }
        }
    }
}

impl Node {
    pub fn new(
        listener: TcpListener,
        proof_backend: Arc<RwLock<BlockFile>>,
        proof_index: Arc<BlocksIndex>,
        view: Arc<ChainView>,
        new_blocks: Receiver<BlockHash>,
        magic: Magic,
    ) -> Self {
        let sockets = Arc::new(RwLock::new(vec![]));
        let closure_sockets = sockets.clone();

        std::thread::spawn(move || {
            Self::notify_loop(new_blocks, closure_sockets, magic);
        });

        Self {
            listener,
            proof_backend,
            proof_index,
            chainview: view,
            sockets,
        }
    }

    pub fn notify_loop(
        new_blocks: Receiver<BlockHash>,
        sockets: Arc<RwLock<Vec<TcpStream>>>,
        magic: Magic,
    ) {
        loop {
            let Ok(block_hash) = new_blocks.recv() else {
                continue;
            };

            info!("New block: {}", block_hash);
            let mut sockets = sockets.write().unwrap();
            sockets.retain(|mut socket| {
                let inv = RawNetworkMessage::new(
                    magic,
                    NetworkMessage::Inv(vec![Inventory::Block(block_hash)]),
                );

                let message = serialize(&inv);
                socket.write_all(&message).is_ok()
            });
        }
    }

    pub fn accept_connections(self) {
        loop {
            if let Ok((stream, addr)) = self.listener.accept() {
                info!("New connection from {}", addr);
                let proof_backend = self.proof_backend.clone();
                let proof_index = self.proof_index.clone();
                self.sockets
                    .write()
                    .unwrap()
                    .push(stream.try_clone().unwrap());

                let peer = Peer::new(
                    stream,
                    addr.to_string(),
                    addr.to_string(),
                    proof_backend,
                    proof_index,
                    self.chainview.clone(),
                );
                std::thread::spawn(move || peer.peer_loop());
            }
        }
    }
}
