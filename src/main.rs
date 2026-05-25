use std::io;

use clap::Parser as _;
use iron::{
    args::Args,
    terminal::Terminal,
};

fn main() -> io::Result<()> {
    #[cfg(feature = "profile")]
    let server_addr = format!("0.0.0.0:{}", puffin_http::DEFAULT_PORT);
    #[cfg(feature = "profile")]
    let _puffin_server = puffin_http::Server::new(&server_addr).unwrap();
    #[cfg(feature = "profile")]
    puffin::set_scopes_on(true);

    Terminal::run(Args::parse())
}
