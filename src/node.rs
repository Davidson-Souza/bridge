//SPDX-License-Identifier: MIT

use std::io::Cursor;
use std::io::Write;
use std::net::TcpListener;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::Mutex;

use bitcoin::consensus::deserialize;
use bitcoin::consensus::Decodable;
use bitcoin::consensus::Encodable;
use bitcoin::hashes::Hash;
use bitcoin::network::constants::ServiceFlags;
use bitcoin::network::message::NetworkMessage;
use bitcoin::network::message::RawNetworkMessage;
use bitcoin::network::message_blockdata::Inventory;
use bitcoin::BlockHash;
use log::info;

use crate::chainview::ChainView;
use crate::blockfile::BlocksFileManager;
use crate::blockfile::BlocksIndex;

pub struct Node {
    listener: TcpListener,
    proof_backend: Arc<Mutex<BlocksFileManager>>,
    proof_index: Arc<BlocksIndex>,
    chainview: Arc<ChainView>,
}
pub struct Peer {
    proof_backend: Arc<Mutex<BlocksFileManager>>,
    reader: TcpStream,
    writer: TcpStream,
    proof_index: Arc<BlocksIndex>,
    chainview: Arc<ChainView>,
}

impl Peer {
    pub fn new(
        stream: TcpStream,
        _peer: String,
        _peer_id: String,
        proof_backend: Arc<Mutex<BlocksFileManager>>,
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
    pub fn handle_request(&mut self) -> Result<(), bitcoin::consensus::encode::Error> {
        let request = RawNetworkMessage::consensus_decode(&mut self.reader)?;
        match request.payload {
            NetworkMessage::Ping(nonce) => {
                let pong = &RawNetworkMessage {
                    magic: request.magic,
                    payload: NetworkMessage::Pong(nonce),
                };
                pong.consensus_encode(&mut self.writer).unwrap();
            }
            NetworkMessage::GetData(inv) => {
                let mut blocks = Cursor::new(vec![]);

                for el in inv {
                    match el {
                        Inventory::UtreexoWitnessBlock(block_hash) => {
                            let block = self.proof_index.get_index(block_hash).unwrap();
                            let mut lock = self.proof_backend.lock().unwrap();
                            match lock.get_block(block) {
                                //TODO: Rust-Bitcoin asks for a block, but we have it serialized on disk already.
                                //      We should be able to just send the block without deserializing it.
                                Some(block) => {
                                    let block = RawNetworkMessage {
                                        magic: request.magic,
                                        payload: NetworkMessage::Block(block),
                                    };
                                    block.consensus_encode(&mut blocks).unwrap();
                                }
                                None => {
                                    let res = RawNetworkMessage {
                                        magic: request.magic,
                                        payload: NetworkMessage::NotFound(vec![
                                            Inventory::WitnessBlock(block_hash),
                                        ]),
                                    };
                                    res.consensus_encode(&mut self.writer).unwrap();
                                }
                            }
                        }
                        Inventory::WitnessBlock(block_hash) => {
                            let block = self.proof_index.get_index(block_hash).unwrap();
                            let mut lock = self.proof_backend.lock().unwrap();

                            match lock.get_block(block) {
                                Some(block) => {
                                    let block = RawNetworkMessage {
                                        magic: request.magic,
                                        payload: NetworkMessage::Block(block.block.into()),
                                    };
                                    block.consensus_encode(&mut blocks).unwrap();
                                }
                                None => {
                                    let res = RawNetworkMessage {
                                        magic: request.magic,
                                        payload: NetworkMessage::NotFound(vec![
                                            Inventory::WitnessBlock(block_hash),
                                        ]),
                                    };
                                    res.consensus_encode(&mut self.writer).unwrap();
                                }
                            }
                        }
                        Inventory::Unknown { hash, .. } => {
                            let block = self
                                .proof_index
                                .get_index(BlockHash::from_inner(hash))
                                .unwrap();
                            let mut lock = self.proof_backend.lock().unwrap();
                            match lock.get_block(block) {
                                Some(block) => {
                                    let block = RawNetworkMessage {
                                        magic: request.magic,
                                        payload: NetworkMessage::Block(block),
                                    };
                                    block.consensus_encode(&mut blocks).unwrap();
                                }
                                None => {
                                    let res = RawNetworkMessage {
                                        magic: request.magic,
                                        payload: NetworkMessage::NotFound(vec![
                                            Inventory::WitnessBlock(BlockHash::from_inner(hash)),
                                        ]),
                                    };
                                    res.consensus_encode(&mut self.writer).unwrap();
                                }
                            }
                        }
                        // TODO: Prove mempool txs
                        _ => {}
                    }
                }
                self.writer.write_all(&blocks.into_inner()).unwrap();
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

                let headers = &RawNetworkMessage {
                    magic: request.magic,
                    payload: NetworkMessage::Headers(headers),
                };
                headers.consensus_encode(&mut self.writer).unwrap();
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
                let our_version = &RawNetworkMessage {
                    magic: request.magic,
                    payload: NetworkMessage::Version(
                        bitcoin::network::message_network::VersionMessage {
                            version: 70001,
                            services: ServiceFlags::NETWORK_LIMITED
                                | ServiceFlags::NETWORK
                                | ServiceFlags::WITNESS
                                | ServiceFlags::from(1 << 24),
                            timestamp: version.timestamp + 1,
                            receiver: version.sender,
                            sender: version.receiver,
                            nonce: version.nonce + 100,
                            user_agent: "/rustreexo:0.1.0/bridge:0.1.0".to_string(),
                            start_height: 800_000,
                            relay: false,
                        },
                    ),
                };

                our_version.consensus_encode(&mut self.writer).unwrap();
                let verack = &RawNetworkMessage {
                    magic: request.magic,
                    payload: NetworkMessage::Verack,
                };
                verack.consensus_encode(&mut self.writer).unwrap();
            }
            _ => {}
        }
        Ok(())
    }
    pub fn peer_loop(mut self) {
        loop {
            if let Err(_) = self.handle_request() {
                info!("Connection closed");
                break;
            }
        }
    }
}

impl<'a> Node {
    pub fn new(
        listener: TcpListener,
        proof_backend: Arc<Mutex<BlocksFileManager>>,
        proof_index: Arc<BlocksIndex>,
        view: Arc<ChainView>,
    ) -> Self {
        Self {
            listener,
            proof_backend,
            proof_index,
            chainview: view,
        }
    }
    pub fn accept_connections(self) {
        while let Ok((stream, addr)) = self.listener.accept() {
            info!("New connection from {}", addr);
            let proof_backend = self.proof_backend.clone();
            let proof_index = self.proof_index.clone();
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
