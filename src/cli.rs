use clap::Parser;

#[derive(Debug, Parser)]
pub struct CliArgs {
    /// If you want to run the bridge from a specific height, you can specify it here.
    ///
    /// Notice that this requires passing the initial state path, that should point to a valid
    /// accumulator state at the specified height.
    #[clap(long, requires("initial-state-path"))]
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
}
