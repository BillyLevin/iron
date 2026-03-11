use std::path::PathBuf;

#[derive(Debug, clap::Parser)]
pub struct Args {
    #[arg(value_hint = clap::ValueHint::FilePath)]
    pub(crate) file_path: PathBuf,
}
