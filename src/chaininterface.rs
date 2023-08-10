//SPDX License Identifier: MIT

//! A trait that abstracts how we pull data from the blockchain. We need some way to get blocks
//! and transactions from the blockchain, but we don't want to do consensus here. So we just
//! provide a trait that can be implemented by something, or at least talks to something
//! that does.

use bitcoin::{Block, BlockHash, BlockHeader, Transaction, Txid};
use bitcoincore_rpc::{Client, RpcApi};

pub trait Blockchain {
    type Error;
    /// Returns the entire content of a block, given a block hash.
    fn get_block(&self, block_hash: BlockHash) -> Result<Block, Self::Error>;
    /// Returns the entire content of a transaction, given a transaction id.
    fn get_transaction(&self, txid: Txid) -> Result<Transaction, Self::Error>;
    /// Returns the block hash of the block at the given height.
    fn get_block_hash(&self, height: u64) -> Result<BlockHash, Self::Error>;
    /// Returns the height of the block with the given hash.
    fn get_block_height(&self, block_hash: BlockHash) -> Result<u32, Self::Error>;
    /// Returns the block header of the block with the given hash.
    fn get_block_header(&self, block_hash: BlockHash) -> Result<BlockHeader, Self::Error>;
    /// Returns how many blocks are in the blockchain.
    fn get_block_count(&self) -> Result<u64, Self::Error>;
    /// Returns the raw transaction info, given a transaction id.
    fn get_raw_transaction_info(&self, txid: &Txid) -> Result<TransactionInfo, Self::Error>;
}

impl Blockchain for Client {
    type Error = bitcoincore_rpc::Error;
    fn get_block(&self, block_hash: BlockHash) -> Result<Block, Self::Error> {
        <Self as RpcApi>::get_block(self, &block_hash)
    }
    fn get_transaction(&self, txid: Txid) -> Result<Transaction, Self::Error> {
        self.get_raw_transaction(&txid, None)
    }
    fn get_block_hash(&self, height: u64) -> Result<BlockHash, Self::Error> {
        <Self as RpcApi>::get_block_hash(self, height)
    }
    fn get_block_height(&self, block_hash: BlockHash) -> Result<u32, Self::Error> {
        let info = self.get_block_info(&block_hash)?;
        Ok(info.height as u32)
    }
    fn get_block_header(&self, block_hash: BlockHash) -> Result<BlockHeader, Self::Error> {
        <Self as RpcApi>::get_block_header(self, &block_hash)
    }
    fn get_block_count(&self) -> Result<u64, Self::Error> {
        <Self as RpcApi>::get_block_count(self)
    }
    fn get_raw_transaction_info(&self, txid: &Txid) -> Result<TransactionInfo, Self::Error> {
        let tx_data = <Self as RpcApi>::get_raw_transaction_info(self, txid, None)?;
        let height = self.get_block_height(tx_data.blockhash.unwrap()).unwrap();
        Ok(TransactionInfo {
            tx: tx_data.transaction().unwrap(),
            height,
            blockhash: tx_data.blockhash,
            is_coinbase: tx_data.is_coinbase(),
        })
    }
}
#[derive(Debug)]
pub struct TransactionInfo {
    pub tx: Transaction,
    pub height: u32,
    pub blockhash: Option<BlockHash>,
    pub is_coinbase: bool,
}
