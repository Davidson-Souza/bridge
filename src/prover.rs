use std::sync::Arc;

use bitcoin::{consensus::deserialize, Block, OutPoint, Transaction, TxIn};
use bitcoincore_rpc::{Client, RpcApi};
use rustreexo::accumulator::{node_hash::NodeHash, pollard::Pollard, proof::Proof};

use crate::{
    prove::{ProofFile, ProofIndex, ProofsIndex},
    udata::LeafData,
};

pub struct Prover<'a> {
    file: ProofFile,
    rpc: Client,
    acc: Pollard,
    storage: Arc<ProofsIndex<'a>>,
    height: u32,
}

impl<'b> Prover<'b> {
    pub fn new(rpc: Client, index_database: Arc<ProofsIndex<'b>>, file: ProofFile) -> Prover<'b> {
        let height = rpc.get_block_count().unwrap() as u32;
        let acc = Pollard::new();

        Self {
            rpc,
            acc,
            height,
            storage: index_database,
            file,
        }
    }
    pub fn prove_range(&mut self, start: u32, end: u32) {
        for height in start..end {
            let block = self.rpc.get_block_hash(height as u64).unwrap();
            let block = self.rpc.get_block(&block).unwrap();
            self.process_block(&block, height);
        }
    }
    fn get_input_leaf_hash(&self, input: &TxIn) -> NodeHash {
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

        let leaf: LeafData = LeafData {
            block_hash: tx_info.blockhash.unwrap(),
            header_code: if tx.is_coin_base() {
                height << 1 | 1
            } else {
                height << 1
            },
            prevout: OutPoint {
                txid: input.previous_output.txid,
                vout: input.previous_output.vout,
            },
            utxo: output.to_owned(),
        };
        leaf.get_leaf_hashes()
    }
    fn save_proof(&mut self, height: u32, proof: Proof) {
        let index = self.file.append(proof);
        self.storage.append(index, height);
    }
    fn process_block(&mut self, block: &Block, height: u32) {
        let mut inputs = Vec::new();
        let mut utxos = Vec::new();

        for tx in block.txdata.iter() {
            let txid = tx.txid();
            for input in tx.input.iter() {
                if !tx.is_coin_base() {
                    let hash = self.get_input_leaf_hash(input);
                    if let Some(idx) = utxos.iter().position(|h| *h == hash) {
                        utxos.remove(idx);
                    } else {
                        inputs.push(self.get_input_leaf_hash(input));
                    }
                }
            }
            for (idx, output) in tx.output.iter().enumerate() {
                if !output.script_pubkey.is_provably_unspendable() {
                    let leaf = LeafData {
                        block_hash: block.block_hash(),
                        header_code: if tx.is_coin_base() {
                            height << 1 | 1
                        } else {
                            height << 1
                        },
                        prevout: OutPoint {
                            txid,
                            vout: idx as u32,
                        },
                        utxo: output.to_owned(),
                    };
                    utxos.push(leaf.get_leaf_hashes());
                }
            }
        }
        let (proof, _) = self.acc.prove(&inputs).unwrap();
        self.acc = self.acc.modify(&utxos, &inputs).unwrap();
        self.save_proof(height, proof);
    }
}
