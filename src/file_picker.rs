use std::{
    cmp,
    fs::File,
    io::{
        self,
        BufRead as _,
        BufReader,
    },
    path::{
        Path,
        PathBuf,
    },
};

use crossterm::event::{
    KeyCode,
    KeyModifiers,
};
use fff_search::{
    FFFMode,
    FilePicker as FFFFilePicker,
    FilePickerOptions,
    FuzzySearchOptions,
    QueryParser,
    SharedFilePicker,
    SharedFrecency,
};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::{
    buffer::Buffer,
    editor::{
        EditorAction,
        EventContext,
        EventOutcome,
    },
    keymap::KeyBinding,
    style::Style,
    text::{
        ByteIndex,
        text_width,
    },
    ui::{
        Columns,
        Dimensions,
        Layer,
        LayerEvent,
        LayerKind,
        Line,
        MaybeBordered,
        Position,
        Rectangle,
        Rows,
        Span,
    },
};

pub(crate) struct FileIndex {
    picker: SharedFilePicker,
    root: PathBuf,
}

impl FileIndex {
    pub(crate) fn new(root: &Path) -> Self {
        let shared_picker = SharedFilePicker::default();
        let shared_frecency = SharedFrecency::noop();

        let _ = FFFFilePicker::new_with_shared_state(
            shared_picker.clone(),
            shared_frecency,
            FilePickerOptions {
                base_path: root.to_string_lossy().to_string(),
                mode: FFFMode::Neovim,
                ..Default::default()
            },
        );

        Self {
            picker: shared_picker,
            root: root.to_path_buf(),
        }
    }

    pub(crate) fn picker(&self) -> SharedFilePicker {
        self.picker.clone()
    }
}

pub(crate) struct FilePicker {
    picker: SharedFilePicker,
    /// The root of the [`FileIndex`].
    root: PathBuf,
    search_term: String,
    cursor_position: Option<Position>,
    files: Vec<RelativeFilePath>,
    /// The index of [`files`](Self::files) that is
    /// currently selected.
    selected_index: usize,
    scroll_offset: Rows,
    layout: Layout,
}

impl FilePicker {
    pub(crate) fn new(file_index: &FileIndex, app_dimensions: Dimensions) -> Self {
        let mut this = Self {
            picker: file_index.picker(),
            root: file_index.root.clone(),
            search_term: String::new(),
            cursor_position: None,
            files: Vec::new(),
            selected_index: 0,
            scroll_offset: Rows::new(0),
            layout: Layout::new(app_dimensions),
        };

        this.update_files();

        this
    }

    fn apply_action(&mut self, action: Action, context: &mut EventContext) {
        match action {
            Action::InsertChar(ch) => {
                self.search_term.push(ch);
                self.update_files();
                self.selected_index = 0;
            }
            Action::RemoveChar => {
                self.search_term.pop();
                self.update_files();
                self.selected_index = 0;
            }
            Action::Close => {
                context.push_action(EditorAction::RemoveLayer(LayerKind::FilePicker));
            }
            Action::GoToNextItem => {
                self.selected_index = (self.selected_index + 1)
                    .checked_rem_euclid(self.files.len())
                    .unwrap_or(0);
            }
            Action::GoToPrevItem => {
                self.selected_index = self
                    .selected_index
                    .checked_sub(1)
                    .unwrap_or_else(|| self.files.len().saturating_sub(1));
            }
            Action::OpenFile => {
                if let Some(path) = self
                    .files
                    .get(self.selected_index)
                    .map(|path| self.root.join(&path.value))
                {
                    context.push_action(EditorAction::OpenFile(path));
                    context.push_action(EditorAction::RemoveLayer(LayerKind::FilePicker));
                }
            }
        }
    }

    fn update_files(&mut self) {
        let Ok(guard) = self.picker.read() else {
            return;
        };

        let Some(picker) = guard.as_ref() else { return };

        self.files = picker
            .fuzzy_search(
                &QueryParser::default().parse(&self.search_term),
                None,
                FuzzySearchOptions {
                    max_threads: 0,
                    ..Default::default()
                },
            )
            .items
            .iter()
            .filter_map(|item| RelativeFilePath::new(item.relative_path(picker)))
            .collect();
    }

    fn horizontal_scroll(&self, text_rectangle: &Rectangle) -> ByteIndex {
        let mut scroll = ByteIndex::new(0);
        let mut width = Columns::new(0);

        for grapheme in self.search_term.graphemes(true).rev() {
            width += text_width(grapheme);
            if width >= text_rectangle.width() {
                scroll += ByteIndex::new(grapheme.len());
            }
        }

        scroll
    }

    /// Highlight substring matches in the file name for the current
    /// [`search_term`](Self::search_term).
    fn decorate_file_path<'file>(
        &self,
        file_path: &'file RelativeFilePath,
        base_style: Style,
    ) -> Vec<Span<'file>> {
        let file_path = &file_path.value;

        file_path.find(&self.search_term).map_or_else(
            || vec![Span::new(file_path).with_style(base_style)],
            |index| {
                let (prefix, rest) = file_path.split_at(index);
                let (matched, suffix) = rest.split_at(self.search_term.len());

                [
                    (prefix, base_style),
                    (matched, base_style.merge(Style::FILTER_MATCH)),
                    (suffix, base_style),
                ]
                .into_iter()
                .filter(|&(chunk, _)| !chunk.is_empty())
                .map(|(chunk, style)| Span::new(chunk).with_style(style))
                .collect()
            },
        )
    }

    fn recalculate_scroll(&mut self) {
        let current_row = Rows::new(self.selected_index);
        let height = self.layout.file_list.inner().height();

        if current_row < self.scroll_offset {
            // upwards scroll
            self.scroll_offset = current_row;
        } else if current_row >= self.scroll_offset + height {
            // downwards scroll
            self.scroll_offset = current_row.saturating_sub(height.saturating_sub(Rows::new(1)));
        } else {
            // no scroll
        }

        self.scroll_offset = cmp::min(current_row, self.scroll_offset);
    }
}

impl Layer for FilePicker {
    fn render(&mut self, buffer: &mut Buffer) {
        debug_assert_eq!(
            self.layout,
            Layout::new(buffer.dimensions()),
            "resizes must be handled correctly"
        );

        self.cursor_position = None;
        self.recalculate_scroll();

        let Layout {
            ref container,
            ref search_input,
            ref file_list,
            ref preview,
        } = self.layout;

        let input = search_input.inner();
        let file_panel = file_list.inner();
        let preview_panel = preview.inner();

        buffer.clear_and_style_rectangle(container, Style::COMMAND_LIST);

        buffer.draw_border(search_input, Style::COMMAND_LIST_BORDER);
        buffer.draw_border(file_list, Style::COMMAND_LIST_BORDER);
        buffer.draw_border(preview, Style::COMMAND_LIST_BORDER);

        let search_text = self
            .search_term
            .get(self.horizontal_scroll(input).value()..)
            .expect("horizontal scroll should give a byte index on a char boundary");

        buffer.render_span(
            &Span::new(search_text).with_style(Style::COMMAND_LIST_INPUT_TEXT),
            &input.offset(),
            input,
        );

        self.cursor_position = Some(input.offset().col_offset(cmp::min(
            text_width(search_text),
            input.width().saturating_sub(1_usize),
        )));

        buffer.render_lines(
            &self
                .files
                .iter()
                .enumerate()
                .skip(self.scroll_offset.value())
                .map(|(i, file_name)| {
                    Line::new(self.decorate_file_path(
                        file_name,
                        if i == self.selected_index {
                            Style::COMMAND_LIST_ITEM_SELECTED
                        } else {
                            Style::COMMAND_LIST_ITEM
                        },
                    ))
                })
                .collect::<Vec<_>>(),
            file_panel,
        );

        buffer.render_lines(
            &self
                .files
                .get(self.selected_index)
                .map_or_else(
                    || Ok(vec![Line::new(vec![Span::new("No file selected")])]),
                    |path| {
                        File::open(self.root.join(&path.value))
                            .map(BufReader::new)?
                            .lines()
                            .take(preview_panel.height().value())
                            .map(|line| line.map(|text| Line::new(vec![Span::new(text)])))
                            .collect::<io::Result<Vec<Line>>>()
                    },
                )
                .unwrap_or_else(|_| vec![Line::new(vec![Span::new("Could not display preview")])]),
            preview_panel,
        );
    }

    fn handle_event(
        &mut self,
        event: &LayerEvent,
        event_context: &mut EventContext,
    ) -> EventOutcome {
        match *event {
            LayerEvent::Key(key_event) => {
                let key_binding = KeyBinding::from(key_event);

                let action = match (key_binding.code(), key_binding.modifiers()) {
                    (KeyCode::Char(ch), KeyModifiers::NONE) => Some(Action::InsertChar(ch)),
                    (KeyCode::Backspace, KeyModifiers::NONE) => Some(Action::RemoveChar),
                    (KeyCode::Esc, KeyModifiers::NONE) => Some(Action::Close),
                    (KeyCode::Down, KeyModifiers::NONE) => Some(Action::GoToNextItem),
                    (KeyCode::Up, KeyModifiers::NONE) => Some(Action::GoToPrevItem),
                    (KeyCode::Enter, _any_modifiers) => Some(Action::OpenFile),
                    _ => None,
                };

                action.map_or(EventOutcome::Unhandled, |a| {
                    self.apply_action(a, event_context);
                    EventOutcome::Handled
                })
            }

            LayerEvent::FocusGained
            | LayerEvent::FocusLost
            | LayerEvent::Mouse(_)
            | LayerEvent::Paste => EventOutcome::Unhandled,
        }
    }

    fn visual_cursor_position(&self) -> Option<Position> {
        self.cursor_position
    }

    fn handle_internal_events(&mut self) -> EventOutcome {
        EventOutcome::Unhandled
    }

    fn kind(&self) -> Option<LayerKind> {
        Some(LayerKind::FilePicker)
    }

    fn resize(&mut self, app_dimensions: Dimensions) {
        self.layout = Layout::new(app_dimensions);
    }
}

#[derive(Debug, Clone)]
struct RelativeFilePath {
    value: String,
}

impl RelativeFilePath {
    fn new(path: String) -> Option<Self> {
        Path::new(&path)
            .is_relative()
            .then_some(Self { value: path })
    }
}

#[derive(Debug, Clone, Copy)]
enum Action {
    InsertChar(char),
    RemoveChar,
    Close,
    GoToNextItem,
    GoToPrevItem,
    OpenFile,
}

#[derive(Debug, PartialEq, Eq)]
struct Layout {
    /// The entire file picker.
    container: Rectangle,
    search_input: MaybeBordered,
    file_list: MaybeBordered,
    preview: MaybeBordered,
}

impl Layout {
    fn new(app_dimensions: Dimensions) -> Self {
        let app_rectangle = Rectangle::from_dimensions(app_dimensions);

        let container = app_rectangle.at_center(Dimensions::new(
            app_rectangle.width() * 8 / 10,
            app_rectangle.height() * 8 / 10,
        ));

        let (search_input, files) = container.split_at_row(Rows::new(3));

        let search_input = MaybeBordered::new(search_input);

        let (files, preview) = files.split_at_column(files.width() / 2);

        let file_list = MaybeBordered::new(files);
        let preview = MaybeBordered::new(preview);

        Self {
            container,
            search_input,
            file_list,
            preview,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use crossterm::event::KeyCode;

    use crate::{
        editor::EventOutcome,
        test_harness::TestHarness,
        ui::{
            Columns,
            Dimensions,
            Rows,
        },
    };

    #[test]
    fn file_picker_opens() {
        let mut harness = TestHarness::with_files(
            "hello\nthere\nworld",
            Dimensions::new(Columns::new(46), Rows::new(12)),
            [
                ("main.rs", "fn main() {}"),
                ("README.md", "# hello"),
                ("Cargo.toml", "[package]\nname = \"iron\""),
            ],
        );

        assert_matches!(harness.send_key(KeyCode::Char(' ')), EventOutcome::Handled);
        assert_matches!(harness.send_key(KeyCode::Char('p')), EventOutcome::Handled);

        insta::assert_snapshot!(
            harness.screen().to_string(),
            @"
        ┌──────▼───────────────────────────────────────┐
        │   1 hello                                    │
        │   1 ┌──────────────────────────────────┐     │
        ▶   2 │                                  │     │
        │     └──────────────────────────────────┘     │
        │     ┌────────────────┐┌────────────────┐     │
        │     │main.rs         ││fn main() {}    │     │
        │     │README.md       ││                │     │
        │     │Cargo.toml      ││                │     │
        │     │test.txt        ││                │     │
        │     └────────────────┘└────────────────┘     │
        │ NORMAL  test.txt                        text │
        │                                              │
        └──────────────────────────────────────────────┘
        "
        );
    }

    #[test]
    fn file_picker_navigates_with_arrows() {
        let mut harness = TestHarness::with_files(
            "hello\nthere\nworld",
            Dimensions::new(Columns::new(46), Rows::new(12)),
            [
                ("main.rs", "fn main() {}"),
                ("README.md", "# hello"),
                ("Cargo.toml", "[package]\nname = \"iron\""),
            ],
        );

        assert_matches!(harness.send_key(KeyCode::Char(' ')), EventOutcome::Handled);
        assert_matches!(harness.send_key(KeyCode::Char('p')), EventOutcome::Handled);

        assert_matches!(harness.send_key(KeyCode::Down), EventOutcome::Handled);
        insta::assert_snapshot!(
            harness.screen().to_string(),
            @"
        ┌──────▼───────────────────────────────────────┐
        │   1 hello                                    │
        │   1 ┌──────────────────────────────────┐     │
        ▶   2 │                                  │     │
        │     └──────────────────────────────────┘     │
        │     ┌────────────────┐┌────────────────┐     │
        │     │main.rs         ││# hello         │     │
        │     │README.md       ││                │     │
        │     │Cargo.toml      ││                │     │
        │     │test.txt        ││                │     │
        │     └────────────────┘└────────────────┘     │
        │ NORMAL  test.txt                        text │
        │                                              │
        └──────────────────────────────────────────────┘
        "
        );

        // wraps around from top to bottom
        assert_matches!(harness.send_key(KeyCode::Up), EventOutcome::Handled);
        assert_matches!(harness.send_key(KeyCode::Up), EventOutcome::Handled);

        insta::assert_snapshot!(
            harness.screen().to_string(),
            @"
        ┌──────▼───────────────────────────────────────┐
        │   1 hello                                    │
        │   1 ┌──────────────────────────────────┐     │
        ▶   2 │                                  │     │
        │     └──────────────────────────────────┘     │
        │     ┌────────────────┐┌────────────────┐     │
        │     │main.rs         ││hello           │     │
        │     │README.md       ││there           │     │
        │     │Cargo.toml      ││world           │     │
        │     │test.txt        ││                │     │
        │     └────────────────┘└────────────────┘     │
        │ NORMAL  test.txt                        text │
        │                                              │
        └──────────────────────────────────────────────┘
        "
        );
    }

    #[test]
    fn file_picker_filters_on_search() {
        let mut harness = TestHarness::with_files(
            "hello\nthere\nworld",
            Dimensions::new(Columns::new(46), Rows::new(12)),
            [
                ("main.rs", "fn main() {}"),
                ("README.md", "# hello"),
                ("Cargo.toml", "[package]\nname = \"iron\""),
            ],
        );

        assert_matches!(harness.send_key(KeyCode::Char(' ')), EventOutcome::Handled);
        assert_matches!(harness.send_key(KeyCode::Char('p')), EventOutcome::Handled);

        let search_term = "cargo";

        for ch in search_term.chars() {
            assert_matches!(harness.send_key(KeyCode::Char(ch)), EventOutcome::Handled);
        }

        insta::assert_snapshot!(
            harness.screen().to_string(),
            @r#"
        ┌───────────▼──────────────────────────────────┐
        │   1 hello                                    │
        │   1 ┌──────────────────────────────────┐     │
        ▶   2 │cargo                             │     │
        │     └──────────────────────────────────┘     │
        │     ┌────────────────┐┌────────────────┐     │
        │     │Cargo.toml      ││[package]       │     │
        │     │                ││name = "iron"   │     │
        │     │                ││                │     │
        │     │                ││                │     │
        │     └────────────────┘└────────────────┘     │
        │ NORMAL  test.txt                        text │
        │                                              │
        └──────────────────────────────────────────────┘
        "#
        );

        for _ in 0..search_term.chars().count() {
            assert_matches!(harness.send_key(KeyCode::Backspace), EventOutcome::Handled);
        }

        insta::assert_snapshot!(
            harness.screen().to_string(),
            @"
        ┌──────▼───────────────────────────────────────┐
        │   1 hello                                    │
        │   1 ┌──────────────────────────────────┐     │
        ▶   2 │                                  │     │
        │     └──────────────────────────────────┘     │
        │     ┌────────────────┐┌────────────────┐     │
        │     │main.rs         ││fn main() {}    │     │
        │     │README.md       ││                │     │
        │     │Cargo.toml      ││                │     │
        │     │test.txt        ││                │     │
        │     └────────────────┘└────────────────┘     │
        │ NORMAL  test.txt                        text │
        │                                              │
        └──────────────────────────────────────────────┘
        "
        );
    }

    #[test]
    fn file_picker_opens_files() {
        let mut harness = TestHarness::with_files(
            "hello\nthere\nworld",
            Dimensions::new(Columns::new(46), Rows::new(12)),
            [
                ("main.rs", "fn main() {}"),
                ("README.md", "# hello\nthis is some markdown"),
                ("Cargo.toml", "[package]\nname = \"iron\""),
            ],
        );

        assert_matches!(harness.send_key(KeyCode::Char(' ')), EventOutcome::Handled);
        assert_matches!(harness.send_key(KeyCode::Char('p')), EventOutcome::Handled);

        assert_matches!(harness.send_key(KeyCode::Down), EventOutcome::Handled);
        assert_matches!(harness.send_key(KeyCode::Enter), EventOutcome::Handled);

        insta::assert_snapshot!(
            harness.screen().to_string(),
            @"
        ┌─────▼────────────────────────────────────────┐
        ▶   1 # hello                                  │
        │   1 this is some markdown                    │
        │                                              │
        │                                              │
        │                                              │
        │                                              │
        │                                              │
        │                                              │
        │                                              │
        │                                              │
        │ NORMAL  README.md                       text │
        │                                              │
        └──────────────────────────────────────────────┘
        "
        );
    }
}
