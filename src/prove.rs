//! We save proofs as a flat file, with a header that contains the number of proofs in the file.
//! Each proof is a list of hashes and targets, we use those to reconstruct the tree up
//! to the root. We save them as a flat blob.

use std::{collections::HashMap, fs::File, io::Seek};

use rustreexo::accumulator::proof::Proof;
/// The number of proofs we save in each file.
const PROOFS_PER_FILE: usize = 1000;

/// Points to a proof in the file.
#[derive(Debug, Clone, Copy)]
pub struct BlockIndex {
    file: u32,
    offset: usize,
}
impl BlockIndex {
    pub fn new(file: u32, offset: usize) -> Self {
        Self { file, offset }
    }
}
#[derive(Debug)]
/// Represents a proof files
pub struct ProofFile {
    file: File,
    id: u32,
    size: u32,
    index: u32,
}

impl ProofFile {
    /// Creates a new proof file.
    pub fn new(file: File) -> Self {
        Self {
            file,
            id: 0,
            size: 0,
            index: 0,
        }
    }

    /// Returns the proof at the given index.
    pub fn get(&mut self, index: BlockIndex) -> Proof {
        self.file
            .seek(std::io::SeekFrom::Start(index.offset as u64))
            .unwrap();
        Proof::deserialize(&self.file).unwrap()
    }
    pub fn append(&mut self, proof: Proof) -> BlockIndex {
        self.file.seek(std::io::SeekFrom::End(0)).unwrap();
        proof.serialize(&mut self.file).unwrap();

        let pos = self.file.stream_position().unwrap();
        let old_pos = self.index;
        self.index = pos as u32;
        BlockIndex {
            file: self.id,
            offset: old_pos as usize,
        }
    }
}

pub struct ProofFileManager {
    /// Hold all files we have open, but not closed yet.
    open_files: Vec<ProofFile>,
    /// Maps a file name to the index of the file in the open_files vector.
    open_files_cache: HashMap<String, usize>,
    /// The number of files we have.
    files: u32,
}

impl ProofFileManager {
    pub fn new() -> Self {
        Self {
            open_files: Vec::new(),
            open_files_cache: HashMap::new(),
            files: 0,
        }
    }
    pub fn get_file(&mut self, file: u32) -> &mut ProofFile {
        let file_name = format!("proofs-{}.dat", file);
        if let Some(index) = self.open_files_cache.get(&file_name) {
            return &mut self.open_files[*index];
        }
        let file = File::open(&file_name).unwrap();
        let mut file = ProofFile::new(file);
        self.open_files.push(file);
        self.open_files_cache
            .insert(file_name, self.open_files.len() - 1);
        let n_files = self.open_files.len() - 1;
        &mut self.open_files[n_files]
    }
    pub fn get_file_iterator(&mut self, file: u32) -> ProofFileIterator {
        let file = self.get_file(file);
        ProofFileIterator::new(file)
    }
    pub fn get_file_count(&self) -> u32 {
        self.files
    }
    pub fn get_proof_count(&self) -> u32 {
        self.open_files.len() as u32 * PROOFS_PER_FILE as u32
    }
    pub fn get_proof(&mut self, index: BlockIndex) -> Proof {
        let file = self.get_file(index.file);
        file.get(index)
    }
    pub fn append(&mut self, proof: Proof) -> BlockIndex {
        let file = self.get_file(self.files);
        let index = file.append(proof);
        self.files += 1;
        index
    }
}

pub struct ProofFileIterator<'a> {
    file: &'a mut ProofFile,
    index: u32,
}

impl<'a> ProofFileIterator<'a> {
    pub fn new(file: &'a mut ProofFile) -> Self {
        Self { file, index: 0 }
    }
}

impl<'a> Iterator for ProofFileIterator<'a> {
    type Item = Proof;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.file.size {
            return None;
        }
        let proof = self.file.get(BlockIndex {
            file: self.file.id,
            offset: self.index as usize,
        });
        self.index += 1;
        Some(proof)
    }
}
pub enum IndexEntry {
    Index(BlockIndex),
}
impl kv::Value for IndexEntry {
    fn from_raw_value(r: kv::Raw) -> Result<Self, kv::Error> {
        Ok(IndexEntry::Index(BlockIndex {
            file: u32::from_be_bytes(r[0..4].try_into().unwrap()),
            offset: usize::from_be_bytes(r[4..].try_into().unwrap()),
        }))
    }
    fn to_raw_value(&self) -> Result<kv::Raw, kv::Error> {
        match self {
            IndexEntry::Index(index) => {
                let mut buf = Vec::new();
                buf.extend_from_slice(&index.file.to_be_bytes());
                buf.extend_from_slice(&index.offset.to_be_bytes());
                Ok(kv::Raw::from(buf))
            }
        }
    }
}
pub struct ProofsIndex {
    pub database: kv::Store,
}

impl ProofsIndex {
    pub fn get_index<'a>(&self, height: u32) -> Option<BlockIndex> {
        let bucket = self
            .database
            .bucket::<&'a [u8], IndexEntry>(Some(&"index"))
            .unwrap();
        let key = height.to_be_bytes();
        match bucket.get(&&*key.as_slice()) {
            Ok(Some(IndexEntry::Index(index))) => Some(index),
            _ => None,
        }
    }
    pub fn append<'a>(&self, index: BlockIndex, height: u32) {
        let bucket = self
            .database
            .bucket::<&'a [u8], IndexEntry>(Some(&"index"))
            .unwrap();
        let key = height.to_be_bytes();
        bucket
            .set(&&key.as_slice(), &IndexEntry::Index(index))
            .expect("Failed to write index");
    }
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;

    use rustreexo::accumulator::node_hash::NodeHash;

    use super::*;

    #[test]
    fn test_proof_file() {
        // Creates two proofs, saves them to a file and the index
        {
            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .read(true)
                .truncate(true)
                .open("test.proofs")
                .unwrap();
            let index_store = ProofsIndex {
                database: kv::Store::new(kv::Config {
                    path: "./index/".into(),
                    temporary: false,
                    use_compression: false,
                    flush_every_ms: None,
                    cache_capacity: None,
                    segment_size: None,
                })
                .unwrap(),
            };

            let mut file = ProofFile::new(file);
            let proof = Proof::new(
                [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13].to_vec(),
                [NodeHash::from([0; 32]), NodeHash::from([0; 32])].to_vec(),
            );
            let index = file.append(proof.clone());
            index_store.append(index, 0);

            let index = file.append(proof.clone());
            index_store.append(index, 1);

            assert_eq!(file.get(index), proof);
        }
        // try to recover the proofs
        let index_store = ProofsIndex {
            database: kv::Store::new(kv::Config {
                path: "./index/".into(),
                temporary: false,
                use_compression: false,
                flush_every_ms: None,
                cache_capacity: None,
                segment_size: None,
            })
            .unwrap(),
        };
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .read(true)
            .open("test.proofs")
            .unwrap();
        let mut file = ProofFile::new(file);
        let index = index_store.get_index(1).unwrap();
        assert_eq!(
            file.get(index),
            Proof::new(
                [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13].to_vec(),
                [NodeHash::from([0; 32]), NodeHash::from([0; 32])].to_vec()
            )
        );
    }
}
