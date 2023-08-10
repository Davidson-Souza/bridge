//SPDX license identifier: MIT

//! Use a Esplora server as a backend to get blocks and transactions. You can use this instead
//! of Bitcoin Core. But keep in mind that while doing the initial sync, it will be slower and
//! you risk getting banned from the server if you do too many requests.

use std::fmt::Display;

use crate::chaininterface::{Blockchain, TransactionInfo};
use bitcoin::{consensus, Block, BlockHash};
use bitcoin_hashes::Hash;
use reqwest::blocking::Client;

#[derive(Debug)]
pub struct EsploraBlockchain {
    client: Client,
    url: String,
}

impl EsploraBlockchain {
    pub fn new(url: String) -> Self {
        Self {
            client: Client::new(),
            url,
        }
    }
}

impl Blockchain for EsploraBlockchain {
    type Error = EsploraError;

    fn get_block_hash(&self, height: u64) -> Result<bitcoin::BlockHash, Self::Error> {
        let url = format!("{}/block-height/{}", self.url, height);
        let block = self.client.get(&url).send()?.text()?;
        Ok(block.parse().unwrap_or(BlockHash::all_zeros()))
    }

    fn get_block(&self, block_hash: BlockHash) -> Result<Block, Self::Error> {
        let url = format!("{}/block/{}/raw", self.url, block_hash);
        let block = self.client.get(&url).send()?.bytes()?;
        Ok(consensus::deserialize::<Block>(&block).unwrap())
    }

    fn get_transaction(&self, txid: bitcoin::Txid) -> Result<bitcoin::Transaction, Self::Error> {
        let url = format!("{}/tx/{}/raw", self.url, txid);
        let tx = self.client.get(&url).send()?.bytes()?;
        Ok(consensus::deserialize::<bitcoin::Transaction>(&tx).unwrap())
    }

    fn get_block_height(&self, block_hash: BlockHash) -> Result<u32, Self::Error> {
        let url = format!("{}/block/{}", self.url, block_hash);
        let block = self.client.get(&url).send()?.text()?;
        let block: serde_json::Value = serde_json::from_str(&block)?;
        Ok(block["height"].as_u64().unwrap() as u32)
    }

    fn get_block_header(&self, block_hash: BlockHash) -> Result<bitcoin::BlockHeader, Self::Error> {
        let url = format!("{}/block/{}/header", self.url, block_hash);
        let header = self.client.get(&url).send()?.text()?;
        let header: serde_json::Value = serde_json::from_str(&header)?;
        let header = hex::decode(header["hex"].as_str().unwrap())?;
        Ok(consensus::deserialize::<bitcoin::BlockHeader>(&header).unwrap())
    }

    fn get_block_count(&self) -> Result<u64, Self::Error> {
        let url = format!("{}/blocks/tip/height", self.url);
        let height = self.client.get(&url).send()?.text()?;
        Ok(height.parse().unwrap())
    }

    fn get_raw_transaction_info(
        &self,
        txid: &bitcoin::Txid,
    ) -> Result<TransactionInfo, Self::Error> {
        let client = Client::new();
        let url = format!("{}/tx/{}/status", self.url, txid);
        let tx = client.get(&url).send().unwrap().text().unwrap();
        let tx: serde_json::Value = serde_json::from_str(&tx).unwrap();

        let tx_hex = client
            .get(&format!("{}/tx/{}/hex", self.url, txid))
            .send()
            .unwrap()
            .text()
            .unwrap();

        Ok(TransactionInfo {
            is_coinbase: tx["vin"][0]["coinbase"].as_str().is_some(),
            blockhash: Some(tx["block_hash"].as_str().unwrap().parse().unwrap()),
            height: tx["block_height"].as_u64().unwrap() as u32,
            tx: consensus::deserialize(&hex::decode(tx_hex).unwrap()).unwrap(),
        })
    }
}
#[derive(Debug)]
pub enum EsploraError {
    Reqwest(reqwest::Error),
    BitcoinCore(bitcoincore_rpc::Error),
    Bitcoin(bitcoin::consensus::encode::Error),
    Hex(hex::FromHexError),
    Json(serde_json::Error),
}

impl From<reqwest::Error> for EsploraError {
    fn from(e: reqwest::Error) -> Self {
        Self::Reqwest(e)
    }
}
impl From<serde_json::Error> for EsploraError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

impl From<bitcoincore_rpc::Error> for EsploraError {
    fn from(e: bitcoincore_rpc::Error) -> Self {
        Self::BitcoinCore(e)
    }
}

impl From<bitcoin::consensus::encode::Error> for EsploraError {
    fn from(e: bitcoin::consensus::encode::Error) -> Self {
        Self::Bitcoin(e)
    }
}

impl From<hex::FromHexError> for EsploraError {
    fn from(e: hex::FromHexError) -> Self {
        Self::Hex(e)
    }
}
impl Display for EsploraError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EsploraError::Reqwest(e) => write!(f, "Reqwest error: {}", e),
            EsploraError::BitcoinCore(e) => write!(f, "Bitcoin Core error: {}", e),
            EsploraError::Bitcoin(e) => write!(f, "Bitcoin error: {}", e),
            EsploraError::Hex(e) => write!(f, "Hex error: {}", e),
            EsploraError::Json(e) => write!(f, "Json error: {}", e),
        }
    }
}
impl std::error::Error for EsploraError {}
#[cfg(test)]
mod tests {
    use crate::chaininterface::TransactionInfo;

    use super::Client;
    use bitcoin::{consensus, Block, BlockHash};

    #[test]
    fn test_get_block() {
        let url = "https://blockstream.info/api/";
        let client = Client::new();
        let blockhash = "00000000000000000000bdd2ec0c94a35d76c6de2ae29e02dd901ac58373f77d"
            .parse::<BlockHash>()
            .unwrap();
        let url = format!("{}/block/{}/raw", url, blockhash);
        let block = client.get(&url).send().unwrap().bytes().unwrap();
        assert_eq!(
            blockhash,
            consensus::deserialize::<Block>(&block)
                .unwrap()
                .block_hash()
        );
    }
    #[test]
    fn test_get_tx_info() {
        let client = Client::new();
        let base_url = "https://mempool.space/signet/api/";
        let url = format!(
            "{}/tx/{}/status",
            base_url, "5c6574473085c1b25fc53d95f85cfdf3b6ba64fffe88893c62bc5bfd99028e89"
        );
        let tx = client.get(&url).send().unwrap().text().unwrap();
        let tx: serde_json::Value = serde_json::from_str(&tx).unwrap();

        let tx_hex = client
            .get(&format!(
                "{}/tx/{}/hex",
                base_url, "5c6574473085c1b25fc53d95f85cfdf3b6ba64fffe88893c62bc5bfd99028e89"
            ))
            .send()
            .unwrap()
            .text()
            .unwrap();

        let _ = TransactionInfo {
            is_coinbase: tx["vin"][0]["coinbase"].as_str().is_some(),
            blockhash: Some(tx["block_hash"].as_str().unwrap().parse().unwrap()),
            height: tx["block_height"].as_u64().unwrap() as u32,
            tx: consensus::deserialize(&hex::decode(tx_hex).unwrap()).unwrap(),
        };
    }
}
