use bitcoin::BlockHash;
use bitcoin::hashes::Hash;

/// We use this index to keep track of information held on flat files. Right now we only store the
/// blocks in the file, but if we build things like compact block filters, we can reuse the
/// indexing logic.
pub enum IndexEntry {
    /// The index of a block in the file.
    Index(BlockIndex),
}

#[derive(Debug)]
/// The index of a block in a file. Each block have an associated index entry.
pub struct BlockIndex {
    /// The offset of the block in the file counted from the file's begginning, in bytes.
    pub offset: usize,
    /// The size of the block in bytes.
    pub size: usize,
}

impl kv::Value for IndexEntry {
    fn from_raw_value(r: kv::Raw) -> Result<Self, kv::Error> {
        Ok(IndexEntry::Index(BlockIndex {
            size: u64::from_be_bytes(r[..8].try_into().unwrap()) as usize,
            offset: u64::from_be_bytes(r[8..].try_into().unwrap()) as usize,
        }))
    }

    fn to_raw_value(&self) -> Result<kv::Raw, kv::Error> {
        match self {
            IndexEntry::Index(index) => {
                let mut buf = Vec::new();
                buf.extend_from_slice(&(index.size as u64).to_be_bytes());
                buf.extend_from_slice(&(index.offset as u64).to_be_bytes());
                Ok(kv::Raw::from(buf))
            }
        }
    }
}

/// An index to help us finding blocks and heights.
///
/// This keeps track of where in the file each block is stored, so we can quickly access them.
pub struct BlocksIndex {
    pub database: kv::Store,
}

impl BlocksIndex {
    /// Returns the index for a block, given its hash. If the block is not found, it returns None.
    pub fn get_index<'a>(&self, block: BlockHash) -> Option<BlockIndex> {
        let bucket = self
            .database
            .bucket::<&'a [u8], IndexEntry>(Some("index"))
            .unwrap();

        let key: [u8; 32] = block.into_inner();
        match bucket.get(&key.as_slice()) {
            Ok(Some(IndexEntry::Index(index))) => Some(index),
            _ => None,
        }
    }

    /// Saves the height of the latest block we have in the index.
    pub fn update_height<'a>(&self, height: usize) {
        let bucket = self
            .database
            .bucket::<&'a [u8], Vec<u8>>(Some("meta"))
            .unwrap();
        let key = b"height";
        bucket
            .set(&key.as_slice(), &height.to_be_bytes().to_vec())
            .expect("Failed to write index");
        bucket.flush().unwrap();
    }

    /// Returns the height of the latest block we have in the index.
    pub fn load_height<'a>(&self) -> usize {
        let bucket = self
            .database
            .bucket::<&'a [u8], Vec<u8>>(Some("meta"))
            .unwrap();
        let key = b"height";
        let height = bucket.get(&key.as_slice());
        match height {
            Ok(Some(height)) => usize::from_be_bytes(height[..].try_into().unwrap()),
            _ => 0,
        }
    }

    /// Appends a new block entry to the index.
    pub fn append<'a>(&self, index: BlockIndex, block: BlockHash) {
        let bucket = self
            .database
            .bucket::<&'a [u8], IndexEntry>(Some("index"))
            .unwrap();
        let key: [u8; 32] = block.into_inner();
        bucket
            .set(&key.as_slice(), &IndexEntry::Index(index))
            .expect("Failed to write index");
    }
}
