use clap::Parser;
use clap::ValueEnum;

#[derive(Debug, Default, Clone, Copy, ValueEnum)]
pub enum Network {
    #[default]
    Mainnet,
    Testnet3,
    Signet,
    Regtest,
}

impl Network {
    pub fn magic(&self) -> u32 {
        match self {
            Network::Mainnet => bitcoin::Network::Bitcoin.magic(),
            Network::Testnet3 => bitcoin::Network::Testnet.magic(),
            Network::Signet => bitcoin::Network::Signet.magic(),
            Network::Regtest => bitcoin::Network::Regtest.magic(),
        }
    }
}

#[derive(Debug, Parser)]
pub struct CliArgs {
    /// If you want to run the bridge from a specific height, you can specify it here.
    ///
    /// Notice that this requires passing the initial state path, that should point to a valid
    /// accumulator state at the specified height.
    #[clap(long, requires("initial_state_path"))]
    pub start_height: Option<u32>,
    /// The path to the initial state file. This file should contain the accumulator state at the
    /// specified height.
    #[clap(long, requires("start_height"))]
    pub initial_state_path: Option<String>,
    /// Creates a snapshot of the accumulator every n blocks
    ///
    /// The file will be named <height>.acc
    #[clap(long)]
    pub acc_snapshot_every_n_blocks: Option<u32>,

    /// In shinigami mode, we save blocks individually in a json file. We also place those json
    /// inside a directory that has a range of blocks (e.g. 0-1000). This parameter specifies the
    /// range of blocks that will be saved in each directory. The default value is 10_000.
    #[clap(long, short = 'g', default_value_t = 10_000)]
    pub block_files_granularity: u32,

    /// If you don't want to save proofs for very old blocks, you can set this options with
    /// the number of blocks you want to keep, and we'll only keep proofs for blocks that are
    /// newer than that.
    #[clap(long)]
    pub save_proofs_after: Option<u32>,

    /// The network we are operating on
    #[clap(long, short = 'n', default_value = "mainnet")]
    pub network: Network,
}
