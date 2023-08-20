//SPDX-License-Identifier: MIT

//! A prover is a thread that keeps up with the blockchain and generates proofs for
//! the utreexo accumulator. Since it holds the entire accumulator, it also provides
//! proofs for other modules. To avoid having multiple channels to and from the prover, it
//! uses a channel to receive requests and sends responses through a oneshot channel, provided
//! by the request sender. Maybe there is a better way to do this, but this is a TODO for later.
use bitcoin::{
    consensus::serialize,
    network::utreexo::{BatchProof, CompactLeafData, ScriptPubkeyType, UData},
    Block, BlockHash, OutPoint, Script, Sequence, Transaction, TxIn, Txid, VarInt, Witness,
};
use bitcoin_hashes::Hash;
use futures::channel::mpsc::Receiver;
use log::info;
use rustreexo::accumulator::{node_hash::NodeHash, pollard::Pollard, proof::Proof};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::{
    chaininterface::Blockchain,
    chainview,
    prove::{BlocksFileManager, BlocksIndex},
    udata::LeafData,
};

/// All the state that the prover needs to keep track of
pub struct Prover {
    /// A reference to a file manager that holds the blocks on disk, using flat files.
    files: Arc<Mutex<BlocksFileManager>>,
    /// A reference to the RPC client that is used to query the blockchain.
    rpc: Box<dyn Blockchain>,
    /// The accumulator that holds the state of the utreexo accumulator.
    acc: Pollard,
    /// An index that keeps track of the blocks that are stored on disk, we need this
    /// to get the blocks from disk.
    storage: Arc<BlocksIndex>,
    /// The height of the blockchain we are on.
    height: u32,
    /// A reference to the chainview, this keeps a map of block hashes to heights and vice versa.
    /// Also keeps block headers for easy access.
    view: Arc<chainview::ChainView>,
    /// A map that keeps track of the leaf data for each outpoint. This is used to generate
    /// proofs for the utreexo accumulator. This is more like a cache, since it won't be
    /// persisted on shutdown.
    leaf_data: HashMap<OutPoint, LeafData>,
}

impl Prover {
    /// Creates a new prover. It loads the accumulator from disk, if it exists.
    pub fn new(
        rpc: Box<dyn Blockchain>,
        index_database: Arc<BlocksIndex>,
        files: Arc<Mutex<BlocksFileManager>>,
        view: Arc<chainview::ChainView>,
    ) -> Prover {
        let height = index_database.load_height() as u32;
        info!("Loaded height {}", height);
        info!("Loading accumulator data...");
        let acc = Self::try_from_disk();
        Self {
            rpc,
            acc,
            height,
            storage: index_database,
            files,
            view,
            leaf_data: HashMap::new(),
        }
    }
    /// Tries to load the accumulator from disk. If it fails, it creates a new one.
    fn try_from_disk() -> Pollard {
        let Ok(file) = std::fs::File::open(crate::subdir("/pollard")) else {
            return Pollard::new();
        };

        let reader = std::io::BufReader::new(file);
        match Pollard::deserialize(reader) {
            Ok(acc) => acc,
            Err(_) => Pollard::new(),
        }
    }
    /// Handles the request from another module. It returns a response through the oneshot channel
    /// provided by the request sender. Errors are returned as strings, maybe this should be changed
    /// to a boxed error or something else.
    fn handle_request(&mut self, req: Requests) -> anyhow::Result<Responses> {
        match req {
            Requests::GetProof(node) => {
                let (proof, _) = self
                    .acc
                    .prove(&[node])
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                Ok(Responses::Proof(proof))
            }
            Requests::GetRoots => {
                let roots = self.acc.get_roots().iter().map(|x| x.get_data()).collect();
                Ok(Responses::Roots(roots))
            }
            Requests::GetBlockByHeight(height) => {
                let hash = self
                    .rpc
                    .get_block_hash(height as u64)
                    .map_err(|_| anyhow::anyhow!("Block at height {} not found", height))?;
                let block = self.storage.get_index(hash).ok_or(anyhow::anyhow!(
                    "Block at height {} not found in storage",
                    height
                ))?;

                let block = self
                    .files
                    .lock()
                    .unwrap()
                    .get_block(block)
                    .ok_or(anyhow::anyhow!(
                        "Block at height {} not found in files",
                        height
                    ))?;
                Ok(Responses::Block(serialize(&block)))
            }
            Requests::GetTransaction(txid) => {
                let tx = self
                    .rpc
                    .get_transaction(txid)
                    .map_err(|_| anyhow::anyhow!("Transaction {} not found", txid))?;
                // TODO: this is a bit of a hack, but it works for now.
                // Rustreexo should have a way to check whether an element is in the
                // pollard. We have this information in the map anyway.
                let hashes: Vec<LeafData> = tx
                    .output
                    .iter()
                    .enumerate()
                    .flat_map(|(idx, _)| {
                        Self::get_full_input_leaf_data(
                            &mut self.leaf_data,
                            &TxIn {
                                previous_output: OutPoint {
                                    txid,
                                    vout: idx as u32,
                                },
                                script_sig: Script::new(),
                                sequence: Sequence::ZERO,
                                witness: Witness::new(),
                            },
                            &self.rpc,
                        )
                    })
                    .filter(|x| self.acc.prove(&[x.get_leaf_hashes()]).is_ok())
                    .collect();
                let (proof, _) = self
                    .acc
                    .prove(
                        &hashes
                            .iter()
                            .map(|x| x.get_leaf_hashes())
                            .collect::<Vec<_>>(),
                    )
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                Ok(Responses::Transaction((tx, proof)))
            }
        }
    }
    /// Gracefully shuts down the prover. It saves the accumulator to disk and flushes the chainview.
    fn shutdown(&mut self) {
        self.save_to_disk();
        self.view.flush();
    }
    /// Saves the accumulator to disk. This is done by serializing the accumulator to a file,
    /// the serialization is done by the rustreexo library and is a depth first traversal of the
    /// tree.
    fn save_to_disk(&self) {
        let file = std::fs::File::create(crate::subdir("/pollard")).unwrap();
        let mut writer = std::io::BufWriter::new(file);
        self.acc.serialize(&mut writer).unwrap();
    }
    /// A infinite loop that keeps the prover up to date with the blockchain. It handles requests
    /// from other modules and updates the accumulator when a new block is found. This method is
    /// also how we create proofs for historical blocks.
    pub fn keep_up(
        &mut self,
        stop: Arc<Mutex<bool>>,
        mut receiver: Receiver<(
            Requests,
            futures::channel::oneshot::Sender<Result<Responses, String>>,
        )>,
    ) -> anyhow::Result<()> {
        let mut last_tip_update = std::time::Instant::now();
        loop {
            if *stop.lock().unwrap() {
                self.shutdown();
                break;
            }
            if let Ok(Some((req, res))) = receiver.try_next() {
                let ret = self.handle_request(req).map_err(|e| e.to_string());
                res.send(ret).unwrap();
            }
            if last_tip_update.elapsed().as_secs() > 10 {
                let height = self.rpc.get_block_count().unwrap() as u32;
                if height > self.height {
                    self.prove_range(self.height + 1, height)?;
                    self.height = height;
                    self.save_to_disk();
                }
                self.storage.update_height(height as usize);
                last_tip_update = std::time::Instant::now();
            }

            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        Ok(())
    }
    /// Proves a range of blocks, may be just one block.
    pub fn prove_range(&mut self, start: u32, end: u32) -> anyhow::Result<()> {
        for height in start..=end {
            let block_hash = self.rpc.get_block_hash(height as u64).unwrap();
            // Update the local index
            self.view.save_block_hash(height, block_hash)?;
            self.view.save_height(block_hash, height)?;
            let block = self.rpc.get_block(block_hash).unwrap();
            self.view
                .save_header(block_hash, serialize(&block.header))?;
            info!("Proving block {}", height);

            let (proof, leaves) = self.process_block(&block, height);
            let block = bitcoin::network::utreexo::UtreexoBlock {
                block,
                udata: Some(UData {
                    remember_idx: vec![],
                    proof,
                    leaves,
                }),
            };
            let index = self.files.lock().unwrap().append(&block, height as usize);
            self.storage.append(index, block.block.block_hash());
        }
        anyhow::Ok(())
    }
    /// Returns what spk type this output is. We need this to build a compact leaf data.
    fn get_spk_type(spk: &Script) -> ScriptPubkeyType {
        if spk.is_p2pkh() {
            ScriptPubkeyType::PubKeyHash
        } else if spk.is_p2sh() {
            ScriptPubkeyType::ScriptHash
        } else if spk.is_v0_p2wpkh() {
            ScriptPubkeyType::WitnessV0PubKeyHash
        } else if spk.is_v0_p2wsh() {
            ScriptPubkeyType::WitnessV0ScriptHash
        } else {
            ScriptPubkeyType::Other(spk.to_bytes().into_boxed_slice())
        }
    }
    /// Pulls the [LeafData] from the bitcoin core rpc. We use this as fallback if we can't find
    /// the leaf in leaf_data. This method is slow and should only be used if we can't find the
    /// leaf in the leaf_data.
    fn get_input_leaf_hash_from_rpc(rpc: &Box<dyn Blockchain>, input: &TxIn) -> Option<LeafData> {
        let tx_info = rpc
            .get_raw_transaction_info(&input.previous_output.txid)
            .ok()?;
        let height = tx_info.height;
        let output = &tx_info.tx.output[input.previous_output.vout as usize];
        let header_code = if tx_info.is_coinbase {
            height << 1 | 1
        } else {
            height << 1
        };
        Some(LeafData {
            block_hash: tx_info.blockhash.unwrap(),
            header_code,
            prevout: OutPoint {
                txid: input.previous_output.txid,
                vout: input.previous_output.vout,
            },
            utxo: output.to_owned(),
        })
    }
    fn get_full_input_leaf_data(
        leaf_data: &mut HashMap<OutPoint, LeafData>,
        input: &TxIn,
        rpc: &Box<dyn Blockchain>,
    ) -> Option<LeafData> {
        leaf_data
            .remove(&input.previous_output)
            .or_else(|| Self::get_input_leaf_hash_from_rpc(rpc, input))
    }
    /// Returns the leaf hash and the compact leaf data for a given input. If the leaf is not in
    /// leaf_data we will try to get it from the bitcoin core rpc.
    fn get_input_leaf_hash(&mut self, input: &TxIn) -> (NodeHash, CompactLeafData) {
        let leaf = self
            .leaf_data
            .remove(&input.previous_output)
            .unwrap_or_else(|| Self::get_input_leaf_hash_from_rpc(&self.rpc, input).unwrap());
        let compact_leaf = CompactLeafData {
            spk_ty: Self::get_spk_type(&leaf.utxo.script_pubkey),
            amount: leaf.utxo.value,
            header_code: leaf.header_code,
        };
        (leaf.get_leaf_hashes(), compact_leaf)
    }
    /// Processes a block and returns the batch proof and the compact leaf data for the block.
    fn process_block(&mut self, block: &Block, height: u32) -> (BatchProof, Vec<CompactLeafData>) {
        let mut inputs = Vec::new();
        let mut utxos = Vec::new();
        let mut compact_leaves = Vec::new();
        for tx in block.txdata.iter() {
            let txid = tx.txid();
            for input in tx.input.iter() {
                if !tx.is_coin_base() {
                    let (hash, compact_leaf) = self.get_input_leaf_hash(input);
                    if let Some(idx) = utxos.iter().position(|h| *h == hash) {
                        utxos.remove(idx);
                    } else {
                        inputs.push(hash);
                        compact_leaves.push(compact_leaf);
                    }
                }
            }
            for (idx, output) in tx.output.iter().enumerate() {
                if !output.script_pubkey.is_provably_unspendable() {
                    let header_code = if tx.is_coin_base() {
                        height << 1 | 1
                    } else {
                        height << 1
                    };
                    let leaf = LeafData {
                        block_hash: block.block_hash(),
                        header_code,
                        prevout: OutPoint {
                            txid,
                            vout: idx as u32,
                        },
                        utxo: output.to_owned(),
                    };
                    utxos.push(leaf.get_leaf_hashes());
                    self.leaf_data.insert(
                        OutPoint {
                            txid,
                            vout: idx as u32,
                        },
                        leaf,
                    );
                }
            }
        }

        let (proof, _) = self.acc.prove(&inputs).unwrap();
        self.acc.modify(&utxos, &inputs).unwrap();
        (
            BatchProof {
                targets: proof.targets.iter().map(|target| VarInt(*target)).collect(),
                hashes: proof
                    .hashes
                    .iter()
                    .map(|hash| BlockHash::from_inner(**hash))
                    .collect(),
            },
            compact_leaves,
        )
    }
}

/// All requests we can send to the prover. The prover will respond with the corresponding
/// response element.
pub enum Requests {
    /// Get the proof for a given leaf hash.
    GetProof(NodeHash),
    /// Get the roots of the accumulator.
    GetRoots,
    /// Get a block at a given height. This method returns the block and utreexo data for it.
    GetBlockByHeight(u32),
    /// Returns a transaction and a proof for all inputs
    GetTransaction(Txid),
}
/// All responses the prover will send.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Responses {
    /// A utreexo proof
    Proof(Proof),
    /// The roots of the accumulator
    Roots(Vec<NodeHash>),
    /// A block and the utreexo data for it, serialized.
    Block(Vec<u8>),
    Transaction((Transaction, Proof)),
}
