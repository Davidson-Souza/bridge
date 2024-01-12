//SPDX-License-Identifier: MIT

use bitcoin::consensus::Decodable;
use bitcoin::consensus::Encodable;
use bitcoin::BlockHash;
use bitcoin::OutPoint;
use bitcoin::TxOut;
use rustreexo::accumulator::node_hash::NodeHash;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha512_256;

/// Leaf data is the data that is hashed when adding to utreexo state. It contains validation
/// data and some commitments to make it harder to attack an utreexo-only node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LeafData {
    /// A commitment to the block creating this utxo
    pub block_hash: BlockHash,
    /// The utxo's outpoint
    pub prevout: OutPoint,
    /// Header code is a compact commitment to the block height and whether or not this
    /// transaction is coinbase. It's defined as
    ///
    /// ```
    /// header_code: u32 = if transaction.is_coinbase() {
    ///     (block_height << 1 ) | 1
    /// } else {
    ///     block_height << 1
    /// };
    /// ```
    pub header_code: u32,
    /// The actual utxo
    pub utxo: TxOut,
}

/// The version tag to be prepended to the leafhash. It's just the sha512 hash of the string
/// `UtreexoV1` represented as a vector of [u8] ([85 116 114 101 101 120 111 86 49]).
/// The same tag is "5574726565786f5631" as a hex string.
pub const UTREEXO_TAG_V1: [u8; 64] = [
    0x5b, 0x83, 0x2d, 0xb8, 0xca, 0x26, 0xc2, 0x5b, 0xe1, 0xc5, 0x42, 0xd6, 0xcc, 0xed, 0xdd, 0xa8,
    0xc1, 0x45, 0x61, 0x5c, 0xff, 0x5c, 0x35, 0x72, 0x7f, 0xb3, 0x46, 0x26, 0x10, 0x80, 0x7e, 0x20,
    0xae, 0x53, 0x4d, 0xc3, 0xf6, 0x42, 0x99, 0x19, 0x99, 0x31, 0x77, 0x2e, 0x03, 0x78, 0x7d, 0x18,
    0x15, 0x6e, 0xb3, 0x15, 0x1e, 0x0e, 0xd1, 0xb3, 0x09, 0x8b, 0xdc, 0x84, 0x45, 0x86, 0x18, 0x85,
];

impl LeafData {
    pub fn get_leaf_hashes(&self) -> NodeHash {
        let mut ser_utxo = vec![];
        let _ = self.utxo.consensus_encode(&mut ser_utxo);
        let leaf_hash = Sha512_256::new()
            .chain_update(UTREEXO_TAG_V1)
            .chain_update(UTREEXO_TAG_V1)
            .chain_update(self.block_hash)
            .chain_update(self.prevout.txid)
            .chain_update(self.prevout.vout.to_le_bytes())
            .chain_update(self.header_code.to_le_bytes())
            .chain_update(ser_utxo)
            .finalize();
        NodeHash::from(leaf_hash.as_slice())
    }
}

impl Decodable for LeafData {
    fn consensus_decode<R: std::io::Read + ?Sized>(
        reader: &mut R,
    ) -> Result<Self, bitcoin::consensus::encode::Error> {
        Self::consensus_decode_from_finite_reader(reader)
    }
    fn consensus_decode_from_finite_reader<R: std::io::Read + ?Sized>(
        reader: &mut R,
    ) -> Result<Self, bitcoin::consensus::encode::Error> {
        let block_hash = BlockHash::consensus_decode(reader)?;
        let prevout = OutPoint::consensus_decode(reader)?;
        let header_code = u32::consensus_decode(reader)?;
        let utxo = TxOut::consensus_decode(reader)?;
        Ok(LeafData {
            block_hash,
            prevout,
            header_code,
            utxo,
        })
    }
}
impl Encodable for LeafData {
    fn consensus_encode<W: std::io::Write + ?Sized>(
        &self,
        writer: &mut W,
    ) -> Result<usize, std::io::Error> {
        let mut len = 0;
        len += self.block_hash.consensus_encode(writer)?;
        len += self.prevout.consensus_encode(writer)?;
        len += self.header_code.consensus_encode(writer)?;
        len += self.utxo.consensus_encode(writer)?;
        Ok(len)
    }
}
