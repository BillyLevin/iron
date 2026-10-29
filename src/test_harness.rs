use std::{
    fmt,
    fs,
    iter,
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
    /// Stored to keep the temp dir alive.
    _dir: TempDir,
}

impl TestHarness {
    pub(crate) fn new(contents: &str, dimensions: Dimensions) -> Self {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join(FILE_NAME);
        fs::write(&file_path, contents).unwrap();

        let editor = Editor::new(file_path, dimensions, None, JJPoller::disabled()).unwrap();

        Self {
            editor,
            buffer: Buffer::new(dimensions),
            _dir: dir,
        }
    }

    pub(crate) fn send_key(&mut self, key: impl Into<KeyEvent>) -> EventOutcome {
        self.editor
            .handle_event(&AppEvent::from(Event::Key(key.into())))
    }

    pub(crate) fn screen(&mut self) -> Screen {
        self.buffer.clear();
        self.editor.render(&mut self.buffer);

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
