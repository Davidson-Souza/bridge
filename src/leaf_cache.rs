use std::collections::HashMap;

use bitcoin::consensus::deserialize;
use bitcoin::consensus::serialize;
use bitcoin::OutPoint;
use kv::Batch;
use kv::Config;
use log::info;

use crate::prover::LeafCache;
use crate::udata::LeafData;

pub struct DiskLeafStorage {
    /// In-memory cache of leaf data
    ///
    /// This is used to avoid hitting the disk database too often,
    /// we put things here until it reaches a certain size, then we
    /// flush it to disk.
    /// If we die before flushing, we'll need txindex to rebuild the
    /// cache.
    cache: HashMap<OutPoint, (u32, LeafData)>,
    /// A disk database of leaf data
    ///
    /// This is used to store leaf data that is not in the cache,
    /// it's a simple kv bucket with no fancy features.
    bucket: kv::Bucket<'static, Vec<u8>, Vec<u8>>,
}

impl LeafCache for DiskLeafStorage {
    fn insert(&mut self, outpoint: OutPoint, leaf_data: LeafData) -> bool {
        let height = leaf_data.header_code >> 1;
        self.cache.insert(outpoint, (height, leaf_data));
        self.cache.len() > 100_000
    }

    fn remove(&mut self, outpoint: &OutPoint) -> Option<LeafData> {
        self.cache
            .remove(outpoint)
            .map(|(_, leaf_data)| leaf_data)
            .or_else(|| {
                let leaf = self.bucket.remove(&serialize(outpoint)).ok().flatten()?;

                deserialize(&leaf).ok()
            })
    }

    fn flush(&mut self) {
        self.flush();
    }

    fn cache_size(&self) -> usize {
        self.cache_size()
    }
}

impl DiskLeafStorage {
    pub fn new(dir: &str) -> Self {
        let db = kv::Store::new(Config {
            cache_capacity: None,
            path: dir.into(),
            flush_every_ms: Some(10000),
            segment_size: None,
            temporary: false,
            use_compression: false,
        })
        .expect("Failed to open leaf cache database");
        let bucket = db.bucket::<Vec<u8>, Vec<u8>>(None).unwrap();
        Self {
            bucket,
            cache: HashMap::with_capacity(100_000),
        }
    }

    fn cache_size(&self) -> usize {
        self.cache.len()
    }

    fn flush(&mut self) {
        info!("Flushing leaf cache to disk, this might take a while");
        let mut new_map = HashMap::new();
        let mut batch = Batch::new();
        for (outpoint, (height, leaf_data)) in self.cache.iter() {
            // Don't uncache things that are too recent
            if *height < 100 {
                new_map.insert(*outpoint, (*height, leaf_data.clone()));
                continue;
            }

            let serialized = serialize(&leaf_data);
            batch
                .set(&serialize(outpoint), &serialized)
                .expect("Failed to flush leaf cache");
        }
        info!("Applying batch to disk, this might take a while");
        self.bucket
            .batch(batch)
            .expect("Failed to flush leaf cache");

        self.cache = new_map;
    }
}
