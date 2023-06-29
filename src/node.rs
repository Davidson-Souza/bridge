use std::{
    io::{Cursor, Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    time::Duration,
};

use bitcoin::{
    consensus::{Decodable, Encodable},
    hashes::Hash,
    network::{
        constants::ServiceFlags,
        message::{NetworkMessage, RawNetworkMessage},
        message_blockdata::Inventory,
    },
    string::FromHexStr,
    BlockHash, CompactTarget, Network,
};
use bitcoincore_rpc::RpcApi;

use crate::{
    blockstore::BlockStore,
    prove::{ProofFileManager, ProofsIndex},
};

pub struct Node {
    listener: TcpListener,
    rpc: Arc<bitcoincore_rpc::Client>,
    proof_backend: Arc<ProofFileManager>,
    proof_index: Arc<ProofsIndex>,
    block_store: Arc<BlockStore>,
}
pub struct Peer {
    proof_backend: Arc<ProofFileManager>,
    peer: String,
    rpc: Arc<bitcoincore_rpc::Client>,
    peer_id: String,
    reader: TcpStream,
    writer: TcpStream,
    proof_index: Arc<ProofsIndex>,
    block_store: Arc<BlockStore>,
}

impl Peer {
    pub fn new(
        stream: TcpStream,
        peer: String,
        peer_id: String,
        proof_backend: Arc<ProofFileManager>,
        proof_index: Arc<ProofsIndex>,
        rpc: Arc<bitcoincore_rpc::Client>,
        block_store: Arc<BlockStore>,
    ) -> Self {
        let reader = stream.try_clone().unwrap();
        Self {
            peer,
            peer_id,
            proof_backend,
            proof_index,
            reader,
            writer: stream,
            rpc,
            block_store,
        }
    }
    pub fn handle_request(&mut self) {
        let request = RawNetworkMessage::consensus_decode(&mut self.reader).unwrap();
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
                        Inventory::WitnessBlock(block_hash) => {
                            let block = self.block_store.fetch_block(&block_hash).unwrap();
                            let block = RawNetworkMessage {
                                magic: request.magic,
                                payload: NetworkMessage::Block(block),
                            };
                            block.consensus_encode(&mut blocks).unwrap();
                        }
                        _ => {}
                    }
                }
                self.writer.write_all(&blocks.into_inner()).unwrap();
            }
            NetworkMessage::GetHeaders(locator) => {
                let mut headers = vec![];
                let mut block = *locator.locator_hashes.first().unwrap();
                for _ in 0..2_000 {
                    let header_info = self.rpc.get_block_header_info(&block).unwrap();
                    headers.push(bitcoin::block::Header {
                        version: header_info.version,
                        prev_blockhash: header_info
                            .previous_block_hash
                            .unwrap_or(BlockHash::all_zeros()),
                        merkle_root: header_info.merkle_root,
                        time: header_info.time as u32,
                        bits: CompactTarget::from_hex_str_no_prefix(&header_info.bits).unwrap(),
                        nonce: header_info.nonce,
                    });

                    let Some(next_block) = header_info.next_block_hash else {break};
                    block = next_block;
                }
                let headers = &RawNetworkMessage {
                    magic: request.magic,
                    payload: NetworkMessage::Headers(headers),
                };
                headers.consensus_encode(&mut self.writer).unwrap();
            }
            NetworkMessage::Version(version) => {
                println!("Version {:?}", version);
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
                            start_height: 0,
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
            NetworkMessage::Unknown { command, .. } => {
                println!("Unknown request {:?}", command);
            }
            _ => {
                println!("Unknown request {:?}", request.payload);
            }
        }
    }
    pub fn peer_loop(mut self) {
        loop {
            self.handle_request();
        }
    }
}

pub trait NodeMethods {
    fn accept_connections(listener: TcpListener);
    fn handle_connection(&self);
    fn peer_loop(&self);
}

impl<'a> Node {
    pub fn new(
        listener: TcpListener,
        proof_backend: Arc<ProofFileManager>,
        proof_index: Arc<ProofsIndex>,
        rpc: Arc<bitcoincore_rpc::Client>,
        block_store: Arc<BlockStore>,
    ) -> Self {
        Self {
            listener,
            proof_backend,
            proof_index,
            rpc,
            block_store,
        }
    }
    pub fn accept_connections(self) {
        while let Ok((stream, addr)) = self.listener.accept() {
            println!("New connection from {}", addr);
            let proof_backend = self.proof_backend.clone();
            let proof_index = self.proof_index.clone();
            let peer = Peer::new(
                stream,
                addr.to_string(),
                addr.to_string(),
                proof_backend,
                proof_index,
                self.rpc.clone(),
                self.block_store.clone(),
            );
            std::thread::spawn(move || peer.peer_loop());
        }
    }
}
