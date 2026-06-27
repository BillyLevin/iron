use std::{
    path::{
        Path,
        PathBuf,
    },
    process::Command,
    sync::{
        self,
        mpsc::{
            Receiver,
            RecvTimeoutError,
            Sender,
        },
    },
    thread,
    time::Duration,
};

/// Sets up a worker thread that polls for information in the current jujutsu
/// workspace.
#[derive(Debug)]
pub(crate) struct JJPoller {
    worker_tx: Sender<PathBuf>,
    result_rx: Receiver<anyhow::Result<JJInfo>>,
}

impl JJPoller {
    pub(crate) fn new(path: PathBuf) -> Self {
        let (worker_tx, worker_rx) = sync::mpsc::channel();
        let (result_tx, result_rx) = sync::mpsc::channel();

        spawn_worker(path, worker_rx, result_tx);

        Self {
            worker_tx,
            result_rx,
        }
    }

    pub(crate) fn refresh(&self) -> Option<JJInfo> {
        if let Ok(mut info) = self.result_rx.try_recv() {
            while let Ok(latest) = self.result_rx.try_recv() {
                info = latest;
            }

            // TODO: is there even any reason to use `Result`? i suppose will be useful
            // for logging if i ever get around to implementing that...
            info.ok()
        } else {
            None
        }
    }

    pub(crate) fn update_path(&self, path: PathBuf) {
        let _ = self.worker_tx.send(path);
    }
}

fn spawn_worker(
    initial_path: PathBuf,
    worker_rx: Receiver<PathBuf>,
    result_tx: Sender<anyhow::Result<JJInfo>>,
) {
    thread::spawn(move || {
        let mut path = initial_path;

        loop {
            if result_tx.send(JJInfo::new(&path)).is_err() {
                break;
            }

            match worker_rx.recv_timeout(Duration::from_secs(1)) {
                Ok(new_path) => {
                    path = worker_rx.try_iter().last().unwrap_or(new_path);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });
}

#[derive(Debug, Default)]
pub(crate) struct JJInfo {
    /// Description of the current jujutsu change in the workspace that the CWD
    /// belongs to.
    description: Option<String>,
    /// Diff stats for the currently opened file if that file is part of the
    /// same workspace that the CWD belongs to.
    diff: Option<DiffSummary>,
}

impl JJInfo {
    fn new(file: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            description: current_change_description()?,
            diff: diff_summary(file)?,
        })
    }

    pub(crate) const fn description(&self) -> Option<&String> {
        self.description.as_ref()
    }

    pub(crate) const fn diff(&self) -> Option<&DiffSummary> {
        self.diff.as_ref()
    }
}

/// Attempts to get the root of the jj workspace for the current working
/// directory.
// TODO: newtype and pass in to any helpers that need it to avoid duplication
pub(crate) fn workspace_root() -> anyhow::Result<PathBuf> {
    let output = Command::new("jj")
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
pub(crate) fn current_change_description() -> anyhow::Result<Option<String>> {
    let output = Command::new("jj")
        .current_dir(workspace_root()?)
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
    let root = workspace_root()?;

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
    lines_added: u32,
    lines_removed: u32,
}

impl DiffSummary {
    pub(crate) const fn added(&self) -> u32 {
        self.lines_added
    }

    pub(crate) const fn removed(&self) -> u32 {
        self.lines_removed
    }
}
