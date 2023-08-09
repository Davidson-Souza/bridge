## Bridge

This is a special propose Bridge node. Bridges are Bitcoin nodes that holds the entire UTXO set in a forest, allowing it to generate proofs in place for blocks and transactions. The goal with this node is provide a simple and efficient tool to generate proofs for the Bitcoin network.

It contains multiple components:
    - A Bitcoin node that accepts connections from other nodes and clients and can serve proofs for blocks and transactions.
    - A REST API that can be used to query the node for proofs.
    - A websocket that serves as lightweight replacement to the bitcoin p2p. It notifies clients about new blocks, sending the necessary proofs.
    - The actual proof generation code.

## Installation

### Requirements

- Rust 1.51.0 or later
- Linux or MacOS
- Because just keep things on RAM, you'll need a machine with at least 16GB of RAM.
- At least 500GB of free disk space, 1TB recommended.
- A trusted source of blocks to connect to. You can use Bitcoin Core.

### From source

```bash
git clone https://github.com/Davidson-Souza/bridge
cd bridge
cargo build --release
```

## Using

### Running the node

This bridge node doesn't perform any validation on the blocks, so you'll need a trusted source of blocks to connect to. You can use the [Bitcoin Core](https://github.com/bitcoin/bitcoin) node to connect to grab blocks from. This is the only dependency of the node.
Assuming you have a Bitcoin node running, just start the bridge as follows:

```bash
./target/release/bridge
```

The bridge will start listening on port 8333 for incoming connections from other nodes and clients. It will also start a websocket server on port 8334. API runs on port 8335. See (the API docs)[docs/API.md] for more information.