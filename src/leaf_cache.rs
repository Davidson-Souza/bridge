use bitcoin::{
    consensus::{deserialize, serialize},
    OutPoint,
};
use kv::Config;

use crate::{prover::LeafCache, udata::LeafData};

pub struct DiskLeafStorage {
    // TODO: Use a in-memory cache to speed up access
    bucket: kv::Bucket<'static, Vec<u8>, Vec<u8>>,
}

impl LeafCache for DiskLeafStorage {
    fn insert(&mut self, outpoint: OutPoint, leaf_data: LeafData) {
        let serialized = serialize(&leaf_data);
        self.bucket
            .set(&serialize(&outpoint), &serialized)
            .expect("Failed to insert leaf into disk cache");
    }
    fn remove(&mut self, outpoint: &OutPoint) -> Option<LeafData> {
        let leaf = self.bucket.remove(&serialize(outpoint)).ok().flatten()?;

        deserialize(&leaf).ok()
    }
}

impl DiskLeafStorage {
    pub fn new(dir: &str) -> Self {
        let db = kv::Store::new(Config {
            cache_capacity: Some(1000),
            path: dir.into(),
            flush_every_ms: Some(1000),
            segment_size: Some(1024 * 1024),
            temporary: false,
            use_compression: false,
        })
        .expect("Failed to open leaf cache database");
        let bucket = db.bucket::<Vec<u8>, Vec<u8>>(None).unwrap();
        Self { bucket }
    }
}
