use std::{
    fmt,
    fs,
    iter,
    path::Path,
    time::{
        Duration,
        SystemTime,
    },
};

use crossterm::event::{
    Event,
    KeyEvent,
};
use tempfile::TempDir;

use crate::{
    buffer::Buffer,
    editor::{
        Editor,
        EventOutcome,
    },
    file_picker::FileIndex,
    jujutsu::JJPoller,
    ui::{
        AppEvent,
        Columns,
        Dimensions,
        Position,
    },
};

const FILE_NAME: &str = "test.txt";

pub(crate) struct TestHarness {
    editor: Editor,
    buffer: Buffer,
    is_closed: bool,

    /// Stored to keep the temp dir alive.
    _dir: TempDir,
}

impl TestHarness {
    pub(crate) fn new(contents: &str, dimensions: Dimensions) -> Self {
        Self::with_files(contents, dimensions, iter::empty())
    }

    /// Create a temporary directory with at least one file (with `contents`),
    /// plus optionally other `files` too.
    pub(crate) fn with_files<'file>(
        contents: &str,
        dimensions: Dimensions,
        files: impl IntoIterator<Item = (&'file str, &'file str)>,
    ) -> Self {
        let dir = TempDir::new().unwrap();

        create_file(
            &dir.path().join(FILE_NAME),
            contents,
            SystemTime::UNIX_EPOCH,
        );

        let files: Vec<_> = files.into_iter().collect();

        for (seconds, (name, file_contents)) in (1..=files.len()).rev().zip(files) {
            create_file(
                &dir.path().join(name),
                file_contents,
                SystemTime::UNIX_EPOCH + Duration::from_secs(seconds as u64),
            );
        }

        let mut this = Self {
            editor: Self::create_editor(&dir, dimensions),
            buffer: Buffer::new(dimensions),
            is_closed: false,
            _dir: dir,
        };

        this.render();
        this
    }

    fn create_editor(dir: &TempDir, dimensions: Dimensions) -> Editor {
        let file_index = FileIndex::new(dir.path());
        assert!(
            file_index.picker().wait_for_scan(Duration::from_secs(5)),
            "file index scan timed out"
        );

        Editor::new(
            dir.path().join(FILE_NAME),
            dimensions,
            None,
            JJPoller::disabled(),
            file_index,
        )
        .unwrap()
    }

    pub(crate) fn send_key(&mut self, key: impl Into<KeyEvent>) {
        assert!(
            !self.is_closed,
            "no more keys should be sent once the editor is closed"
        );

        match self
            .editor
            .handle_event(&AppEvent::from(Event::Key(key.into())))
        {
            EventOutcome::Handled => self.render(),
            EventOutcome::Unhandled => {}
            EventOutcome::CloseApp => self.is_closed = true,
        }
    }

    fn render(&mut self) {
        self.buffer.clear();
        self.editor.render(&mut self.buffer);
    }

    pub(crate) const fn is_closed(&self) -> bool {
        self.is_closed
    }

    pub(crate) fn screen(&self) -> Screen {
        let width = self.buffer.dimensions().width().value();

        let rows = self
            .buffer
            .cells()
            .chunks(width)
            .map(|cells| {
                iter::successors(cells.first().map(|cell| (0, cell)), |&(index, cell)| {
                    let next = index + cell.width().value();
                    cells.get(next).map(|next_cell| (next, next_cell))
                })
                .map(|(_, cell)| cell.content())
                .collect()
            })
            .collect();

        Screen {
            rows,
            width: self.buffer.dimensions().width(),
            cursor: self.editor.visual_cursor_position(),
        }
    }
}

/// Creates and writes to a file. Also sets the given `modified` time because
/// FFF orders items with the same search score based on their modified time, so
/// this allows us to make this deterministic for tests.
fn create_file(path: &Path, contents: &str, modified: SystemTime) {
    fs::write(path, contents).unwrap();
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
}

/// Representation of the rendered [`Editor`]. Does not have style information.
/// The [`fmt::Display`] implementation draws a border to make whitespace easier
/// to see, plus indicates the cursor column and row with "▼" and "▶"
/// respectively.
pub(crate) struct Screen {
    rows: Vec<String>,
    width: Columns,
    cursor: Position,
}

impl fmt::Display for Screen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let width = self.width.value();
        let cursor_col = self.cursor.left().value();
        let cursor_row = self.cursor.top().value();

        let top: String = (0..width)
            .map(|col| if col == cursor_col { '▼' } else { '─' })
            .collect();
        writeln!(f, "┌{top}┐")?;

        for (index, row) in self.rows.iter().enumerate() {
            let left = if index == cursor_row { '▶' } else { '│' };
            writeln!(f, "{left}{row}│")?;
        }

        write!(f, "└{}┘", "─".repeat(width))
    }
}
