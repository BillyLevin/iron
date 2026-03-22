use std::io;

use clap::Parser as _;
use iron::{
    args::Args,
    terminal::Terminal,
};

fn main() -> io::Result<()> {
    let args = Args::parse();

    Terminal::run(&args)
}
