use std::{
    fs,
    path::{
        Path,
        PathBuf,
    },
    process::Command,
};

use anyhow::Context as _;

/// Attempts to get the root of the jj workspace that the given `file` belongs
/// to.
pub(crate) fn workspace_root(file: &Path) -> anyhow::Result<PathBuf> {
    let canonical_path = fs::canonicalize(file)?;
    let current_dir = canonical_path
        .parent()
        .context("could not get parent directory")?;

    let output = Command::new("jj")
        .current_dir(current_dir)
        .arg("root")
        .arg("--ignore-working-copy")
        .arg("--no-integrate-operation")
        .output()?;

    if !output.status.success() {
        anyhow::bail!(
            "`jj root` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(PathBuf::from(str::from_utf8(&output.stdout)?.trim()))
}

/// Attempts to get the description of the current jj change in the workspace
/// that the given `file` belongs to.
pub(crate) fn current_change_description(file: &Path) -> anyhow::Result<Option<String>> {
    let root = workspace_root(file)?;
    let output = Command::new("jj")
        .current_dir(root)
        .arg("log")
        .arg("--ignore-working-copy")
        .arg("--no-integrate-operation")
        .arg("--no-pager")
        .arg("--no-graph")
        .arg("--color=never")
        .arg("--revision=@")
        .arg("--template=self.description().first_line()")
        .output()?;

    if !output.status.success() {
        anyhow::bail!(
            "`jj log` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    match String::from_utf8(output.stdout)?.trim() {
        "" => Ok(None),
        desc => Ok(Some(desc.to_owned())),
    }
}
