use bitcoin::{
    consensus::{deserialize, serialize},
    network::utreexo::{BatchProof, CompactLeafData, ScriptPubkeyType, UData},
    Block, BlockHash, OutPoint, Script, Transaction, TxIn, VarInt,
};
use bitcoin_hashes::Hash;
use bitcoincore_rpc::{Client, RpcApi};
use futures::channel::mpsc::Receiver;
use rustreexo::accumulator::{node_hash::NodeHash, pollard::Pollard, proof::Proof};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::{
    chainview,
    prove::{BlocksFileManager, BlocksIndex},
    udata::LeafData,
};
pub struct Prover {
    files: Arc<Mutex<BlocksFileManager>>,
    rpc: Client,
    acc: Pollard,
    storage: Arc<BlocksIndex>,
    height: u32,
    view: Arc<chainview::ChainView>,
    leaf_data: HashMap<OutPoint, LeafData>,
}

impl Prover {
    pub fn new(
        rpc: Client,
        index_database: Arc<BlocksIndex>,
        files: Arc<Mutex<BlocksFileManager>>,
        view: Arc<chainview::ChainView>,
    ) -> Prover {
        let height = index_database.load_height() as u32;
        println!("Loaded height {}", height);
        print!("Loading accumulator data...");
        let acc = Self::try_from_disk();
        println!("(Done)");
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
    fn try_from_disk() -> Pollard {
        let Ok(file) = std::fs::File::open("pollard") else {
            return Pollard::new();
        };

        let reader = std::io::BufReader::new(file);
        match Pollard::deserialize(reader) {
            Ok(acc) => acc,
            Err(_) => Pollard::new(),
        }
    }
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
        }
    }
    fn shutdown(&mut self) {
        self.save_to_disk();
        self.view.flush();
    }
    fn save_to_disk(&self) {
        let file = std::fs::File::create("pollard").unwrap();
        let mut writer = std::io::BufWriter::new(file);
        self.acc.serialize(&mut writer).unwrap();
    }
    pub fn keep_up(
        &mut self,
        stop: Arc<Mutex<bool>>,
        mut receiver: Receiver<(
            Requests,
            futures::channel::oneshot::Sender<Result<Responses, String>>,
        )>,
    ) -> anyhow::Result<()> {
        loop {
            if *stop.lock().unwrap() {
                self.shutdown();
                break;
            }
            if let Ok(Some((req, res))) = receiver.try_next() {
                let ret = self.handle_request(req).map_err(|e| e.to_string());
                res.send(ret).unwrap();
            }
            let height = self.rpc.get_block_count().unwrap() as u32;
            if height > self.height {
                self.prove_range(self.height + 1, height)?;
                self.height = height;
                self.save_to_disk();
            }
            self.storage.update_height(height as usize);
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        Ok(())
    }
    pub fn prove_range(&mut self, start: u32, end: u32) -> anyhow::Result<()> {
        for height in start..=end {
            let block_hash = self.rpc.get_block_hash(height as u64).unwrap();
            self.view.save_block_hash(height, block_hash)?;
            self.view.save_height(block_hash, height)?;
            let block = self.rpc.get_block(&block_hash).unwrap();
            self.view
                .save_header(block_hash, serialize(&block.header))?;
            println!("Proving block {}", height);

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
    fn get_input_leaf_hash_from_rpc(&self, input: &TxIn) -> LeafData {
        let tx_info = self
            .rpc
            .get_raw_transaction_info(&input.previous_output.txid, None)
            .expect("Bitcoin core isn't working");
        let tx: Transaction = deserialize(&tx_info.hex).unwrap();
        let output = tx.output[input.previous_output.vout as usize].clone();
        let height = self
            .rpc
            .get_block_header_info(&tx_info.blockhash.unwrap())
            .unwrap()
            .height as u32;
        let header_code = if tx.is_coin_base() {
            height << 1 | 1
        } else {
            height << 1
        };
        LeafData {
            block_hash: tx_info.blockhash.unwrap(),
            header_code,
            prevout: OutPoint {
                txid: input.previous_output.txid,
                vout: input.previous_output.vout,
            },
            utxo: output.to_owned(),
        }
    }
    fn get_input_leaf_hash(&mut self, input: &TxIn) -> (NodeHash, CompactLeafData) {
        let leaf = self
            .leaf_data
            .remove(&input.previous_output)
            .unwrap_or_else(|| self.get_input_leaf_hash_from_rpc(input));
        let compact_leaf = CompactLeafData {
            spk_ty: Self::get_spk_type(&leaf.utxo.script_pubkey),
            amount: leaf.utxo.value,
            header_code: leaf.header_code,
        };
        (leaf.get_leaf_hashes(), compact_leaf)
    }

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

pub enum Requests {
    GetProof(NodeHash),
    GetRoots,
    GetBlockByHeight(u32),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Responses {
    Proof(Proof),
    Roots(Vec<NodeHash>),
    Block(Vec<u8>),
}
