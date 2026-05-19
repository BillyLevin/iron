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

/// Attempts to get the diff stats for the given `file`.
pub(crate) fn diff_summary(file: &Path) -> anyhow::Result<Option<DiffSummary>> {
    let root = workspace_root(file)?;

    let relative_path = file
        .strip_prefix(&root)
        .map_or_else(|_| file.to_string_lossy(), |f| f.to_string_lossy());

    let output = Command::new("jj")
        .current_dir(root)
        .arg("log")
        .arg("--no-pager")
        .arg("--no-graph")
        .arg("--color=never")
        .arg("--revision=@")
        .arg(
            format!(r#"--template=self.diff().stat().files().filter(|f| f.display_diff_path() == "{relative_path}").map(|f| f.lines_added() ++ " " ++ f.lines_removed())"#),
        )
        .output()?;

    if !output.status.success() {
        anyhow::bail!(
            "`jj log` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let diff_str = String::from_utf8(output.stdout)?;
    let Some((added, removed)) = diff_str.split_once(' ') else {
        return Ok(None);
    };

    Ok(Some(DiffSummary {
        lines_added: added.parse()?,
        lines_removed: removed.parse()?,
    }))
}

#[derive(Debug)]
pub(crate) struct DiffSummary {
    lines_added: u64,
    lines_removed: u64,
}

impl DiffSummary {
    pub(crate) const fn added(&self) -> u64 {
        self.lines_added
    }

    pub(crate) const fn removed(&self) -> u64 {
        self.lines_removed
    }
}
