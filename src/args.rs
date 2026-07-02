use std::path::PathBuf;

#[derive(Debug, clap::Parser)]
pub struct Args {
    #[arg(value_hint = clap::ValueHint::FilePath)]
    pub(crate) file_path: PathBuf,

    #[arg(short, long, value_enum, default_value_t = LogLevel::Warn, help = "Log level")]
    pub(crate) log_level: LogLevel,
}

impl Args {
    #[must_use]
    pub const fn log_level(&self) -> LogLevel {
        self.log_level
    }
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
#[value(rename_all = "UPPER")]
pub enum LogLevel {
    Warn,
    Info,
    Debug,
    Trace,
}

impl From<LogLevel> for log::LevelFilter {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Warn => Self::Warn,
            LogLevel::Info => Self::Info,
            LogLevel::Debug => Self::Debug,
            LogLevel::Trace => Self::Trace,
        }
    }
}
