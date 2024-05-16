// SPDX-License-Identifier: MIT

//! This module holds all blocks and proofs in a file that gets memory-mapped to the process's address space.
//! This allows for fast access to the data without having to read it from disk, giving the OS the
//! oportunity to cache the data in memory. This also allows for accessing the data in a read-only
//! manner without having to use a mutex to synchronize access to the file.

use std::fs::File;
use std::fs::OpenOptions;
use std::io::Seek;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::slice;

use bitcoin::consensus::Decodable;
use bitcoin::consensus::Encodable;
use bitcoin::hashes::Hash;
use bitcoin::network::utreexo::UtreexoBlock as Block;
use bitcoin::BlockHash;
use mmap::MapOption;
use mmap::MemoryMap;

/// A file that holds all blocks and proofs in a memory-mapped file.
pub struct BlockFile {
    /// A pointer for the memory-mapped region.
    mmap: MemoryMap,
    /// The file that holds the data.
    file: File,
    /// The current position of the writer in the file.
    writer_pos: usize,
}

unsafe impl Send for BlockFile {}
unsafe impl Sync for BlockFile {}

impl BlockFile {
    /// Creates a new memory-mapped file with the given path and size.
    pub fn new(path: PathBuf, map_size: usize) -> Result<Self, std::io::Error> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)?;

        let pos = file.seek(std::io::SeekFrom::End(0))?;
        let mmap = MemoryMap::new(
            map_size,
            &[
                MapOption::MapReadable,
                MapOption::MapReadable,
                MapOption::MapFd(file.as_raw_fd()),
            ],
        )
        .unwrap();

        Ok(Self {
            mmap,
            writer_pos: pos as usize,
            file,
        })
    }
    
    /// Returns a block at the given position.
    pub fn get_block(&self, index: BlockIndex) -> Option<Block> {
        unsafe {
            Block::consensus_decode(&mut slice::from_raw_parts(self.read(&index), index.size)).ok()
        }
    }
    
    /// Appends a block to the file and returns the index of the block.
    pub fn append(&mut self, block: &Block) -> BlockIndex {
        // seek to the end of the file
        self.file.seek(std::io::SeekFrom::End(0)).unwrap();
        let size = block.consensus_encode(&mut self.file).unwrap();
        self.writer_pos += size;

        BlockIndex {
            offset: self.writer_pos - size,
            size,
        }
    }
    
    /// Returns a pointer to the block at the given index.
    ///
    /// This funcion is unsafe because it returns a raw pointer to the memory-mapped region.
    pub unsafe fn read(&self, index: &BlockIndex) -> *mut u8 {
        self.mmap.data().wrapping_add(index.offset)
    }
}

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
