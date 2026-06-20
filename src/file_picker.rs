use std::cmp;

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
    files: Vec<String>,
}

impl FilePicker {
    pub(crate) fn new(picker: SharedFilePicker) -> Self {
        let mut this = Self {
            picker,
            search_term: String::new(),
            cursor_position: None,
            files: Vec::new(),
        };

        this.update_files();

        this
    }

    fn apply_action(&mut self, action: Action, context: &mut EventContext) {
        match action {
            Action::InsertChar(ch) => {
                self.search_term.push(ch);
                self.update_files();
            }
            Action::RemoveChar => {
                self.search_term.pop();
                self.update_files();
            }
            Action::Close => {
                context.push_action(EditorAction::RemoveLayer(LayerKind::FilePicker));
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
            .map(|item| item.relative_path(picker))
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
}

impl Layer for FilePicker {
    fn render(&mut self, buffer: &mut Buffer) {
        let app_rectangle = Rectangle::from_dimensions(buffer.dimensions());

        let rectangle = app_rectangle.at_center(Dimensions::new(
            app_rectangle.width() * 8 / 10,
            app_rectangle.height() * 8 / 10,
        ));

        buffer.clear_and_style_rectangle(&rectangle, Style::COMMAND_LIST);

        let (input_rectangle, file_list_rectangle) = rectangle.split_at_row(Rows::new(3));

        let input_rectangle = buffer.draw_border(&input_rectangle, Style::COMMAND_LIST_BORDER);

        let text = self
            .search_term
            .get(self.horizontal_scroll(&input_rectangle).value()..)
            .expect("horizontal scroll should give a byte index on a char boundary");

        buffer.render_span(
            &Span::new(text).with_style(Style::COMMAND_LIST_INPUT_TEXT),
            &input_rectangle.offset(),
            &input_rectangle,
        );

        let file_list_rectangle =
            buffer.draw_border(&file_list_rectangle, Style::COMMAND_LIST_BORDER);

        buffer.render_lines(
            self.files
                .iter()
                .map(|file| Line::new(vec![Span::new(file)]))
                .collect(),
            &file_list_rectangle,
        );

        self.cursor_position = Some(input_rectangle.offset().col_offset(cmp::min(
            text_width(text),
            input_rectangle.width() - Columns::new(1),
        )));
    }

    fn handle_event(&mut self, event: &Event, event_context: &mut EventContext) -> EventOutcome {
        match *event {
            Event::Key(key_event) => {
                let key_binding = KeyBinding::from(key_event);

                let action = match (key_binding.code(), key_binding.modifiers()) {
                    (KeyCode::Char(ch), KeyModifiers::NONE) => Some(Action::InsertChar(ch)),
                    (KeyCode::Backspace, KeyModifiers::NONE) => Some(Action::RemoveChar),
                    (KeyCode::Esc, KeyModifiers::NONE) => Some(Action::Close),
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

#[derive(Debug, Clone, Copy)]
enum Action {
    InsertChar(char),
    RemoveChar,
    Close,
}
