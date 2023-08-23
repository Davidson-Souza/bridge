//SPDX-License-Identifier: MIT

//! We save proofs as a flat file, with a header that contains the number of proofs in the file.
//! Each proof is a list of hashes and targets, we use those to reconstruct the tree up
//! to the root. We save them as a flat blob.

use crate::subdir;
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::Seek,
};

use bitcoin::{
    consensus::{Decodable, Encodable},
    hashes::Hash,
    network::utreexo::UtreexoBlock as Block,
    BlockHash,
};
use log::info;
/// The number of proofs we save in each file.
const PROOFS_PER_FILE: usize = 1000;

/// Points to a proof in the file.
#[derive(Debug, Clone, Copy)]
pub struct BlockIndex {
    file: u32,
    offset: usize,
}

#[derive(Debug)]
/// Represents a proof files
pub struct BlockFile {
    file: File,
    id: u32,
    _size: u32,
    index: u32,
}

impl BlockFile {
    /// Creates a new proof file.
    pub fn new(mut file: File) -> Self {
        let offset = file.seek(std::io::SeekFrom::End(0)).unwrap();
        Self {
            file,
            id: 0,
            _size: 0,
            index: offset as u32,
        }
    }

    /// Returns the proof at the given index.
    pub fn get(&mut self, index: BlockIndex) -> Result<Block, bitcoin::consensus::encode::Error> {
        self.file
            .seek(std::io::SeekFrom::Start(index.offset as u64))
            .unwrap();
        Block::consensus_decode(&mut self.file)
    }
    pub fn append(&mut self, proof: &Block) -> BlockIndex {
        self.file.seek(std::io::SeekFrom::End(0)).unwrap();
        proof.consensus_encode(&mut self.file).unwrap();

        let pos = self.file.stream_position().unwrap();
        let old_pos = self.index;
        self.index = pos as u32;
        BlockIndex {
            file: self.id,
            offset: old_pos as usize,
        }
    }
}

pub struct BlocksFileManager {
    /// Hold all files we have open, but not closed yet.
    open_files: Vec<BlockFile>,
    /// Maps a file name to the index of the file in the open_files vector.
    open_files_cache: HashMap<String, usize>,
}

impl BlocksFileManager {
    pub fn new() -> Self {
        std::fs::DirBuilder::new()
            .recursive(true) // If recursive is false, if the dir exists, it will fail.
            .create(subdir("blocks"))
            .unwrap();
        Self {
            open_files: Vec::new(),
            open_files_cache: HashMap::new(),
        }
    }
    pub fn get_file(&mut self, file: u32) -> &mut BlockFile {
        let file_name = format!("{}/blocks-{}.dat", subdir("blocks"), file);
        if let Some(index) = self.open_files_cache.get(&file_name) {
            return &mut self.open_files[*index];
        }
        if self.open_files.len() >= 5 {
            info!("Releasing file {}", format!("{}/blocks-{}.dat", subdir("blocks"), file));

            self.open_files.remove(self.open_files.len() - 1);
        }
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .read(true)
            .open(&file_name)
            .unwrap();
        let file = BlockFile::new(file);
        self.open_files.push(file);
        self.open_files_cache
            .insert(file_name, self.open_files.len() - 1);
        let n_files = self.open_files.len() - 1;
        &mut self.open_files[n_files]
    }

    pub fn get_block(&mut self, index: BlockIndex) -> Option<Block> {
        let file = self.get_file(index.file);
        file.get(index).ok()
    }
    pub fn append(&mut self, block: &Block, height: usize) -> BlockIndex {
        let file_number = height as u32 / PROOFS_PER_FILE as u32;
        let file = self.get_file(file_number);

        let mut index = file.append(block);
        index.file = file_number;
        index
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
pub struct BlocksIndex {
    pub database: kv::Store,
}

impl BlocksIndex {
    pub fn get_index<'a>(&self, block: BlockHash) -> Option<BlockIndex> {
        let bucket = self
            .database
            .bucket::<&'a [u8], IndexEntry>(Some(&"index"))
            .unwrap();
        let key: [u8; 32] = block.into_inner();
        match bucket.get(&&*key.as_slice()) {
            Ok(Some(IndexEntry::Index(index))) => Some(index),
            _ => None,
        }
    }
    pub fn update_height<'a>(&self, height: usize) {
        let bucket = self
            .database
            .bucket::<&'a [u8], Vec<u8>>(Some(&"meta"))
            .unwrap();
        let key = b"height";
        bucket
            .set(&&key.as_slice(), &height.to_be_bytes().to_vec())
            .expect("Failed to write index");
        bucket.flush().unwrap();
    }
    pub fn load_height<'a>(&self) -> usize {
        let bucket = self
            .database
            .bucket::<&'a [u8], Vec<u8>>(Some(&"meta"))
            .unwrap();
        let key = b"height";
        let height = bucket.get(&&key.as_slice());
        match height {
            Ok(Some(height)) => usize::from_be_bytes(height[..].try_into().unwrap()),
            _ => 0,
        }
    }
    pub fn append<'a>(&self, index: BlockIndex, block: BlockHash) {
        let bucket = self
            .database
            .bucket::<&'a [u8], IndexEntry>(Some(&"index"))
            .unwrap();
        let key: [u8; 32] = block.into_inner();
        bucket
            .set(&&key.as_slice(), &IndexEntry::Index(index))
            .expect("Failed to write index");
    }
}
