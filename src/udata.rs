// SPDX-License-Identifier: MIT

use bitcoin::BlockHash;
use bitcoin::Script;
use bitcoin::Txid;

#[derive(Debug, Clone)]
pub struct LeafContext {
    pub block_hash: BlockHash,
    pub txid: Txid,
    pub vout: u32,
    pub value: u64,
    pub pk_script: Script,
    pub block_height: u32,
    pub median_time_past: u32,
    pub is_coinbase: bool,
}

#[cfg(not(feature = "shinigami"))]
pub mod bitcoin_leaf_data {
    use bitcoin::consensus::Decodable;
    use bitcoin::consensus::Encodable;
    use bitcoin::BlockHash;
    use bitcoin::OutPoint;
    use bitcoin::TxOut;
    use rustreexo::accumulator::node_hash::BitcoinNodeHash;
    use serde::Deserialize;
    use serde::Serialize;
    use sha2::Digest;
    use sha2::Sha512_256;

    use super::LeafContext;

    /// Leaf data is the data that is hashed when adding to utreexo state. It contains validation
    /// data and some commitments to make it harder to attack an utreexo-only node.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct BitcoinLeafData {
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
        0x5b, 0x83, 0x2d, 0xb8, 0xca, 0x26, 0xc2, 0x5b, 0xe1, 0xc5, 0x42, 0xd6, 0xcc, 0xed, 0xdd,
        0xa8, 0xc1, 0x45, 0x61, 0x5c, 0xff, 0x5c, 0x35, 0x72, 0x7f, 0xb3, 0x46, 0x26, 0x10, 0x80,
        0x7e, 0x20, 0xae, 0x53, 0x4d, 0xc3, 0xf6, 0x42, 0x99, 0x19, 0x99, 0x31, 0x77, 0x2e, 0x03,
        0x78, 0x7d, 0x18, 0x15, 0x6e, 0xb3, 0x15, 0x1e, 0x0e, 0xd1, 0xb3, 0x09, 0x8b, 0xdc, 0x84,
        0x45, 0x86, 0x18, 0x85,
    ];

    impl BitcoinLeafData {
        pub fn get_leaf_hashes(leaf: &LeafContext) -> BitcoinNodeHash {
            let leaf_data = BitcoinLeafData::from(leaf.clone());
            leaf_data.compute_hash()
        }

        fn compute_hash(&self) -> BitcoinNodeHash {
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
            BitcoinNodeHash::from(leaf_hash.as_slice())
        }
    }

    impl Decodable for BitcoinLeafData {
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
            Ok(BitcoinLeafData {
                block_hash,
                prevout,
                header_code,
                utxo,
            })
        }
    }

    impl Encodable for BitcoinLeafData {
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

    impl From<LeafContext> for BitcoinLeafData {
        fn from(value: LeafContext) -> Self {
            BitcoinLeafData {
                block_hash: value.block_hash,
                prevout: OutPoint {
                    txid: value.txid,
                    vout: value.vout,
                },
                header_code: value.block_height << 1 | value.is_coinbase as u32,
                utxo: TxOut {
                    value: value.value,
                    script_pubkey: value.pk_script,
                },
            }
        }
    }
}

#[cfg(feature = "shinigami")]
pub mod shinigami_udata {
    use bitcoin::consensus::Encodable;
    use bitcoin::Script;
    use bitcoin::Txid;
    use bitcoin_hashes::Hash;
    use rustreexo::accumulator::node_hash::AccumulatorHash;
    use serde::Serialize;
    use starknet_crypto::poseidon_hash_many;
    use starknet_crypto::Felt;

    use super::LeafContext;

    pub struct ByteArray {
        pub data: Vec<Felt>,
        pub pending_word: Felt,
        pub pending_word_len: usize,
    }

    pub struct ShinigamiLeafData {
        pub txid: Txid,
        pub vout: u32,
        pub value: u64,
        pub pk_script: ByteArray,
        pub block_height: u32,
        pub median_time_past: u32,
        pub is_coinbase: bool,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    /// We need a stateful wrapper around the actual hash, this is because we use those different
    /// values inside our accumulator. Here we use an enum to represent the different states, you
    /// may want to use a struct with more data, depending on your needs.
    pub enum PoseidonHash {
        /// This means this holds an actual value
        ///
        /// It usually represents a node in the accumulator that haven't been deleted.
        Hash(Felt),
        /// Placeholder is a value that haven't been deleted, but we don't have the actual value.
        /// The only thing that matters about it is that it's not empty. You can implement this
        /// the way you want, just make sure that [BitcoinNodeHash::is_placeholder] and [NodeHash::placeholder]
        /// returns sane values (that is, if we call [BitcoinNodeHash::placeholder] calling [NodeHash::is_placeholder]
        /// on the result should return true).
        Placeholder,
        /// This is an empty value, it represents a node that was deleted from the accumulator.
        ///
        /// Same as the placeholder, you can implement this the way you want, just make sure that
        /// [BitcoinNodeHash::is_empty] and [NodeHash::empty] returns sane values.
        Empty,
    }

    // you'll need to implement Display for your hash type, so you can print it.
    impl std::fmt::Display for PoseidonHash {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                PoseidonHash::Hash(h) => write!(f, "Hash({})", h),
                PoseidonHash::Placeholder => write!(f, "Placeholder"),
                PoseidonHash::Empty => write!(f, "Empty"),
            }
        }
    }

    // this is the implementation of the BitcoinNodeHash trait for our custom hash type. And it's the only
    // thing you need to do to use your custom hash type with the accumulator data structures.
    impl AccumulatorHash for PoseidonHash {
        // returns a new placeholder type such that is_placeholder returns true
        fn placeholder() -> Self {
            PoseidonHash::Placeholder
        }

        // returns an empty hash such that is_empty returns true
        fn empty() -> Self {
            PoseidonHash::Empty
        }

        // returns true if this is a placeholder. This should be true iff this type was created by
        // calling placeholder.
        fn is_placeholder(&self) -> bool {
            matches!(self, PoseidonHash::Placeholder)
        }

        // returns true if this is an empty hash. This should be true iff this type was created by
        // calling empty.
        fn is_empty(&self) -> bool {
            matches!(self, PoseidonHash::Empty)
        }

        // used for serialization, writes the hash to the writer
        //
        // if you don't want to use serialization, you can just return an error here.
        fn write<W>(&self, writer: &mut W) -> std::io::Result<()>
        where
            W: std::io::Write,
        {
            match self {
                PoseidonHash::Hash(h) => writer.write_all(&h.to_bytes_be()),
                PoseidonHash::Placeholder => writer.write_all(&[0u8; 32]),
                PoseidonHash::Empty => writer.write_all(&[0u8; 32]),
            }
        }

        // used for deserialization, reads the hash from the reader
        //
        // if you don't want to use serialization, you can just return an error here.
        fn read<R>(reader: &mut R) -> std::io::Result<Self>
        where
            R: std::io::Read,
        {
            let mut buf = [0u8; 32];
            reader.read_exact(&mut buf)?;
            if buf.iter().all(|&b| b == 0) {
                Ok(PoseidonHash::Empty)
            } else {
                Ok(PoseidonHash::Hash(Felt::from_bytes_be(&buf)))
            }
        }

        // the main thing about the hash type, it returns the next node's hash, given it's children.
        // The implementation of this method is highly consensus critical, so everywhere should use the
        // exact same algorithm to calculate the next hash. Rustreexo won't call this method, unless
        // **both** children are not empty.
        fn parent_hash(left: &Self, right: &Self) -> Self {
            if let (PoseidonHash::Hash(left), PoseidonHash::Hash(right)) = (left, right) {
                return PoseidonHash::Hash(poseidon_hash_many(&[*left, *right]));
            }

            // This should never happen, since rustreexo won't call this method unless both children
            // are not empty.
            unreachable!()
        }
    }

    impl Serialize for PoseidonHash {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            match self {
                PoseidonHash::Hash(h) => {
                    let inner = h.to_raw();
                    inner.serialize(serializer)
                }
                PoseidonHash::Placeholder => serializer.serialize_none(),
                PoseidonHash::Empty => serializer.serialize_none(),
            }
        }
    }

    impl Encodable for PoseidonHash {
        fn consensus_encode<W: std::io::Write + ?Sized>(
            &self,
            writer: &mut W,
        ) -> Result<usize, std::io::Error> {
            match self {
                PoseidonHash::Hash(h) => {
                    let inner = h.to_bytes_be();
                    inner.consensus_encode(writer)
                }
                PoseidonHash::Placeholder => Ok(0),
                PoseidonHash::Empty => Ok(0),
            }
        }
    }

    fn convert_hash256_to_felt(hash: [u8; 32]) -> [Felt; 2] {
        let (hight, low) = hash.split_at(16);

        [
            Felt::from_bytes_be_slice(hight),
            Felt::from_bytes_be_slice(low),
        ]
    }

    fn convert_spk_to_byte_array(pk_script: &Script) -> ByteArray {
        let mut iter = pk_script.as_bytes().chunks_exact(31);
        let mut data = vec![];

        while let Some(chunk) = iter.next() {
            let mut word = [0u8; 31];
            word.copy_from_slice(chunk);
            data.push(Felt::from_bytes_be_slice(&word));
        }

        let pending_word = iter.remainder();
        let pending_word_len = pending_word.len();
        let pending_word = if pending_word_len > 0 {
            let mut word = [0u8; 31];
            word[..pending_word_len].copy_from_slice(pending_word);
            Felt::from_bytes_be_slice(&word)
        } else {
            Felt::from(0u64)
        };

        ByteArray {
            data,
            pending_word,
            pending_word_len,
        }
    }

    impl ShinigamiLeafData {
        pub fn get_leaf_hashes(data: &LeafContext) -> PoseidonHash {
            // rouding up to the next multiple of 2
            let mut data_to_hash = Vec::with_capacity(16);
            let pk_script = convert_spk_to_byte_array(&data.pk_script);
            let mut txid = data.txid.as_hash().into_inner();
            txid.reverse();

            data_to_hash.extend(convert_hash256_to_felt(txid));
            data_to_hash.push(Felt::from(data.vout));
            data_to_hash.push(Felt::from(data.value));
            data_to_hash.extend(pk_script.data);
            data_to_hash.push(pk_script.pending_word);
            data_to_hash.push(Felt::from(pk_script.pending_word_len as u64));
            data_to_hash.push(Felt::from(data.block_height));
            data_to_hash.push(Felt::from(data.median_time_past));
            data_to_hash.push(Felt::from(data.is_coinbase as u64));

            let leaf_hash = poseidon_hash_many(&data_to_hash);

            PoseidonHash::Hash(leaf_hash)
        }
    }
}

#[cfg(not(feature = "shinigami"))]
pub use bitcoin_leaf_data::BitcoinLeafData as LeafData;
#[cfg(feature = "shinigami")]
pub use shinigami_udata::ShinigamiLeafData as LeafData;
