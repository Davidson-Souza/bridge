use std::sync::Arc;

use kv::Store;

pub struct BlockStore {
    blocks_index: Store,
}

impl BlockStore {
    pub fn new(path: &str) -> Self {
        let blocks_index = Store::new(kv::Config {
            path: path.into(),
            temporary: false,
            use_compression: false,
            flush_every_ms: None,
            cache_capacity: None,
            segment_size: None,
        })
        .unwrap();
        Self { blocks_index }
    }
    pub fn put_block<'a>(&self, block: &bitcoin::Block) {
        let block_hash = block.block_hash();
        let block = bitcoin::consensus::serialize(block);
        let bucket = self
            .blocks_index
            .bucket::<'a, &'a [u8], Vec<u8>>(Some(&"blocks"))
            .unwrap();
        bucket.set(&&block_hash[..], &block).unwrap();
    }
    pub fn fetch_block<'a>(&self, block_hash: &bitcoin::BlockHash) -> Option<bitcoin::Block> {
        let bucket = self
            .blocks_index
            .bucket::<'a, &'a [u8], Vec<u8>>(Some(&"blocks"))
            .unwrap();
        match bucket.get(&&block_hash[..]) {
            Ok(Some(block)) => Some(bitcoin::consensus::deserialize(&block).unwrap()),
            _ => None,
        }
    }
}
