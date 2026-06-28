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
    Event,
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
        LayerKind,
        Line,
        Position,
        Rectangle,
        Rows,
        Span,
    },
};

pub(crate) struct FileIndex {
    picker: SharedFilePicker,
}

impl FileIndex {
    pub(crate) fn new() -> Self {
        let shared_picker = SharedFilePicker::default();
        let shared_frecency = SharedFrecency::noop();

        let _ = FFFFilePicker::new_with_shared_state(
            shared_picker.clone(),
            shared_frecency,
            FilePickerOptions {
                base_path: ".".into(),
                mode: FFFMode::Neovim,
                ..Default::default()
            },
        );

        Self {
            picker: shared_picker,
        }
    }

    pub(crate) fn picker(&self) -> SharedFilePicker {
        self.picker.clone()
    }
}

pub(crate) struct FilePicker {
    picker: SharedFilePicker,
    search_term: String,
    cursor_position: Option<Position>,
    files: Vec<RelativeFilePath>,
    /// The index of [`files`](Self::files) that is
    /// currently selected.
    selected_index: usize,
    scroll_offset: Rows,
}

impl FilePicker {
    pub(crate) fn new(picker: SharedFilePicker) -> Self {
        let mut this = Self {
            picker,
            search_term: String::new(),
            cursor_position: None,
            files: Vec::new(),
            selected_index: 0,
            scroll_offset: Rows::new(0),
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
                    .map(|path| PathBuf::from(path.value.clone()))
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

    fn recalculate_scroll(&mut self, file_list_rectangle: &Rectangle) {
        let current_row = Rows::new(self.selected_index);
        let height = file_list_rectangle.height();

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
        let app_rectangle = Rectangle::from_dimensions(buffer.dimensions());

        let rectangle = app_rectangle.at_center(Dimensions::new(
            app_rectangle.width() * 8 / 10,
            app_rectangle.height() * 8 / 10,
        ));

        buffer.clear_and_style_rectangle(&rectangle, Style::COMMAND_LIST);

        let (input_rectangle, list_rectangle) = rectangle.split_at_row(Rows::new(3));

        let input_rectangle = buffer.draw_border(&input_rectangle, Style::COMMAND_LIST_BORDER);

        let search_text = self
            .search_term
            .get(self.horizontal_scroll(&input_rectangle).value()..)
            .expect("horizontal scroll should give a byte index on a char boundary");

        buffer.render_span(
            &Span::new(search_text).with_style(Style::COMMAND_LIST_INPUT_TEXT),
            &input_rectangle.offset(),
            &input_rectangle,
        );

        self.cursor_position = Some(input_rectangle.offset().col_offset(cmp::min(
            text_width(search_text),
            input_rectangle.width() - Columns::new(1),
        )));

        let (list_rectangle, preview_rectangle) =
            list_rectangle.split_at_column(list_rectangle.width() / 2);

        let list_rectangle = buffer.draw_border(&list_rectangle, Style::COMMAND_LIST_BORDER);

        self.recalculate_scroll(&list_rectangle);

        buffer.render_lines(
            self.files
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
                .collect(),
            &list_rectangle,
        );

        let preview_rectangle = buffer.draw_border(&preview_rectangle, Style::COMMAND_LIST_BORDER);

        buffer.render_lines(
            self.files
                .get(self.selected_index)
                .map_or_else(
                    || Ok(vec![Line::new(vec![Span::new("No file selected")])]),
                    |path| {
                        File::open(&path.value)
                            .map(BufReader::new)?
                            .lines()
                            .take(preview_rectangle.height().value())
                            .map(|line| line.map(|text| Line::new(vec![Span::new(text)])))
                            .collect::<io::Result<Vec<Line>>>()
                    },
                )
                .unwrap_or_else(|_| vec![Line::new(vec![Span::new("Could not display preview")])]),
            &preview_rectangle,
        );
    }

    fn handle_event(&mut self, event: &Event, event_context: &mut EventContext) -> EventOutcome {
        match *event {
            Event::Key(key_event) => {
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

            Event::FocusGained
            | Event::FocusLost
            | Event::Mouse(_)
            | Event::Paste(_)
            | Event::Resize(_, _) => EventOutcome::Unhandled,
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
