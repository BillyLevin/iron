use std::{
    fs,
    time::SystemTime,
};

use anyhow::Context as _;
use clap::Parser as _;
use iron::{
    args::{
        Args,
        LogLevel,
    },
    terminal::Terminal,
};

fn main() -> anyhow::Result<()> {
    #[cfg(feature = "profile")]
    let server_addr = format!("0.0.0.0:{}", puffin_http::DEFAULT_PORT);
    #[cfg(feature = "profile")]
    let _puffin_server = puffin_http::Server::new(&server_addr).unwrap();
    #[cfg(feature = "profile")]
    puffin::set_scopes_on(true);

    let args = Args::parse();
    init_logging(args.log_level())?;
    Terminal::run(args)?;

    Ok(())
}

fn init_logging(level: LogLevel) -> anyhow::Result<()> {
    let dirs = directories::ProjectDirs::from("", "", "iron")
        .context("failed to determine project dirs")?;

    let log_file = dirs
        .state_dir()
        .unwrap_or_else(|| dirs.data_local_dir())
        .join("iron.log");

    fs::create_dir_all(
        log_file
            .parent()
            .context("log file must have a parent directory")?,
    )?;

    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{} {} [{}]: {}",
                humantime::format_rfc3339_millis(SystemTime::now()),
                record.target(),
                record.level(),
                message
            ));
        })
        .level(log::LevelFilter::from(level))
        .chain(fern::log_file(&log_file)?)
        .apply()
        .context("failed to init logging")
}
