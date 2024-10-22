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
}
