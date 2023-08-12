This Bridge software contains a REST API that allows users to interact with the Utreexo functionality. The API is server on port 8080 by default. This document describes the API endpoints and their functionality.

## Summary
- [GET /roots](#get-roots)
- [GET /proof/{leaf_hash}](#get-proofleaf_hash)
- [GET /block/{block_height}](#get-blockblock_height)
- [GET /tx/{hash}/ouputs](#get-txhashouputs)

Planned:
- GET /tx/{txid}
- GET /tx/{txid}/proof
- GET /acccumulator
- GET /signed-state (this is like /roots but with a signature from the bridge's owner)

- POST /batch-proof
- POST /update-proof
- POST /tx

## GET /roots
Returns the current state of the accumulator. This includes the root hash, the number of leaves. You can use this to perform a lightweight bootstrap. It is recommended that you maintains the accumulator by yourself, but this endpoint can be used to bootstrap the accumulator.

### Example
```
$ curl localhost:8080/roots
{
    "data":[
        "c3e31033dba3957ffee968e8baa9976078b32e944d014e012f89aaa76826208e","0143ee36351e8264b8d3fd64e45a1376bbd459f0add231f53bf817e165b7c5cd","8ca430dd51625ccced5ddf1fcee9a5eb445096c3c2f9664bd7f14f7b5097e298","871f8ced3d93a83f5b6946e1643ff40eb6a7c3b95cb5997d4207378b3a0a82cb","1b48a4ec2dc44e92355d8750600244bf25255c39552118d1e2cb7eff2ebffa30","64d4c60bb8c10acb3f912917856d0385fabe02b477e271b17e41407231265967","8820d6d7c3594d7405f1b15be842196ed79c91d78e24709ffe9dff1054b0611a","03b31dc8384784c8da57043a2bbef3a6461e9ce49fd223778206579e3382e474","0bb0c8300575487f2f5491f50b1fc0e48a08f6bde369c3a98d76f9a5e523ddf3","60e08645a4fbac3c9f490d783d787db97865e9a111e77a456ac5a52d0f6c1f3c","0e0b3330f6fbcad8b6a9fe167b7ece1b4b2688441e1f5eed87ab70777fe35e5c","6588f8b2ac4728020d63d001b4b2aa42e0c410daa769be2b2ec554fb8ad47f41","ee4e6c6e081b9e8dbb2c236fda06319d741e91ee0167f54e8907cabe95af8dcc","66aef23c6f023ffad1f522ef9f82d02cf1078a9398f15e7d5e5f053b00da0354","2a35d900c992cbe9bbcb26362c63940e93812578d6dc445fabe4bbcb9d9857e3","d0f53e5dd76ca1f91210327387ba24edfd0ae11441421589ae645c7ddc1c4789"
    ],
    "error":null
}
```

## GET /proof/{leaf_hash}
Returns the proof for the given leaf hash. The leaf hash is how we represent leaves in the accumulator, and is the sha256 hash of the serialized leaf data, defined as follows:

```
leaf_data = <block_hash: []bytes[32]><prevout: Outpoint><header_code: u32><Utxo: TxOut>
leaf_hash = sha256(leaf_data)
```
Both prevout and Utxo are serialized using the default Bitcoin serialization format. header_code is defined as block_height << 1 | (is_coinbase ? 1 : 0). The block_height is the height of the block that the transaction is in. is_coinbase is a boolean that is true if the transaction is a coinbase, 0 otherwhise.

The proof is returned as a list of hashes and a list of targets. The targets are the positions of each leaf in the Accumulator, in this case it's only one. The hashes are the hashes needed to prove this leaf. The proof can be verified by hashing up to the root. The result should be one of the roots' hash.

### Example

```
curl localhost:8080/prove/05f5aa68a199c08a72b1fdac6ebe585371982d0e83498539a655bcbad7ed41f2
{
    "data":{
        "hashes":[
            "e59bbbe93412522dc972c217ebbe79ba06d9001a2e6dd152159dfaf52d328ef9","fbf299791df7546fbc9401c2bde1e058a07c8bb79bac173f41293c1a93a716fd","e5d73471c8144ab44df26af0e423821db0bc7dca1a47ed8e357ba12bca71f1a6","a4fa840576e276a4eebcdf19f94bc697dac97b92ba5b16156a137dc16a7600e4","6eb5dafb514f63eb802690d8718a2d478279fb18c70dae6a243b0580736add05","c5ceef1958042fb2ec410258f0cd16c0cde6924dd0a4743e7f08f208a5d48f58","9451f93290b32e19d937fc8d9fd4218150b1407598bcab51cf699550a995fb7b","81a908b59110197ffe759d5ddabb188b81046f1f6ec8d4690b699d971eeb3fcc","b58b6467ccab988378ff15aeaeba42b45ab876a901961cc6505d1d16c4a3f871"
        ],
        "targets":[5983697]
    },
    "error":null
}
```

## GET /block/{block_height}
Returns the block hash for the given block height. Along with the normal block data, we also send the data needed to update the accumulator, like a batch proof and preimages for the inputs. The blocks is serialized as a hex string, using the same format as the Bitcoin Wire protocol.

### Example

```
$ curl localhost:8080/block/1
{
    "data": "00000020f61eee3b63a380a477a063af32b2bbc97c9ff9f01f2c4225e973988108000000f575c83235984e7dc4afc1f30944c170462e84437ab6f2d52e16878a79e4678bd1914d5fae77031eccf4070001010000000001010000000000000000000000000000000000000000000000000000000000000000ffffffff025151feffffff0200f2052a010000001600149243f727dd5343293eb83174324019ec16c2630f0000000000000000776a24aa21a9ede2f61c3f71d1defd3fa999dfa36953755c690689799962b48bebd836974e8cf94c4fecc7daa2490047304402205e423a8754336ca99dbe16509b877ef1bf98d008836c725005b3c787c41ebe46022047246e4467ad7cc7f1ad98662afcaf14c115e0095a227c7b05c5182591c23e7e0100012000000000000000000000000000000000000000000000000000000000000000000000000000000000",
    "error":null
}
```

## GET /tx/{block_height}/outputs

Returns the list of UTXOs created at this transaction and the proof for it. If any output is spent, it won't be proved.

### Example

```bash
$curl http://localhost:8080/tx f757cab0229313c843d93943c41a26823c8439be001a8179bbd0b9f27d1e2d89/outputs
{
  "data": {
    "proof": {
      "hashes": [
        "0938d5c1c06281744d4a00643cd7b8aadd009749e906044482a5233e9275c664",
        "3e5f1f7679a1cf00e3ab6438d4f8a5abfc493732a460dad32096df33c6d55c1b",
        "8f02c9b052c50898ea154dd8f18e9cb9706ba215528d432d2c8f97d09a6bac51",
        "674b3f05ffda0c0fcf9bac97e6df14eba6a61cc5d8d937b76d0219db2f064f4e",
        "239beadd1239f430271c18e9056f7958fe6e339abad69daf635868c7c2444ed4",
        "27c1c89302ff1a492d4107c8fd6876138966abc7c30b89d66fe0540623e449cc",
        "638cecae791f27821fb6af9ae4e9f02cbdca3dc6b86c806b6976e531b7a03566"
      ],
      "targets": [
        3580287
      ]
    },
    "tx": {
      "input": [
        {
          "previous_output": "0000000000000000000000000000000000000000000000000000000000000000:4294967295",
          "script_sig": "036c60020a2f7369676e65743a332f",
          "sequence": 4294967294,
          "witness": [
            "0000000000000000000000000000000000000000000000000000000000000000"
          ]
        }
      ],
      "lock_time": 0,
      "output": [
        {
          "script_pubkey": "51207099e4b23427fc40ba4777bbf52cfd0b7444d69a3e21ef281270723f54c0c14b",
          "value": 5000000000
        },
        {
          "script_pubkey": "6a24aa21a9ede2f61c3f71d1defd3fa999dfa36953755c690689799962b48bebd836974e8cf94c4fecc7daa2490047304402202b2d782c3a26b6529d0d503e00447e8eabf4039908929315fba2ce5b217392dd0220549787d18146131b65a1920336331e3c0807043e6bd64a85131022551e2044120100",
          "value": 0
        }
      ],
      "version": 2
    }
  },
  "error": null
}
```
