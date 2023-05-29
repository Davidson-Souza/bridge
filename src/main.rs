mod udata;

use bitcoin::{consensus::deserialize, Block, OutPoint, Transaction, TxIn};
use bitcoincore_rpc::{Auth, Client, RpcApi};

use rustreexo::accumulator::{node_hash::NodeHash, pollard::Pollard};
use udata::LeafData;
fn main() {
    let cookie = env!("HOME").to_owned() + "/.bitcoin/signet/.cookie";
    let client = Client::new("localhost:38332".into(), Auth::CookieFile(cookie.into())).unwrap();
    let mut acc = Pollard::new();
    for i in 1..=50_000 {
        println!("Indexing block {i}");
        let block = client
            .get_block(&client.get_block_hash(i).unwrap())
            .unwrap();
        acc = process_block(&client, &block, i as u32, acc);
    }
    println!(
        "{:?}",
        acc.get_roots()
            .iter()
            .map(|h| h.get_data().to_string())
            .collect::<Vec<String>>()
    );
    println!("{}", acc.leaves);
}
fn get_input_leaf_hash(client: &Client, input: &TxIn) -> NodeHash {
    let tx_info = client
        .get_raw_transaction_info(&input.previous_output.txid, None)
        .expect("Bitcoin core isn't working");
    let tx: Transaction = deserialize(&tx_info.hex).unwrap();
    let output = tx.output[input.previous_output.vout as usize].clone();
    let height = client
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

fn process_block(client: &Client, block: &Block, height: u32, acc: Pollard) -> Pollard {
    let mut inputs = Vec::new();
    let mut utxos = Vec::new();

    for tx in block.txdata.iter() {
        let txid = tx.txid();
        for input in tx.input.iter() {
            if !tx.is_coin_base() {
                let hash = get_input_leaf_hash(client, input);
                if let Some(idx) = utxos.iter().position(|h| *h == hash) {
                    utxos.remove(idx);
                } else {
                    inputs.push(get_input_leaf_hash(client, input));
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
    acc.modify(&utxos, &inputs).unwrap()
}
