use crossterm::event::Event;
use fff_search::{
    FFFMode,
    FilePicker as FFFFilePicker,
    FilePickerOptions,
    FuzzySearchOptions,
    QueryParser,
    SharedFilePicker,
    SharedFrecency,
};

use crate::{
    buffer::Buffer,
    editor::{
        EventContext,
        EventOutcome,
    },
    style::Style,
    ui::{
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
}

impl FilePicker {
    pub(crate) const fn new(picker: SharedFilePicker) -> Self {
        Self { picker }
    }

    fn files(&self) -> Vec<Line<'_>> {
        let Ok(guard) = self.picker.read() else {
            return Vec::new();
        };

        let Some(picker) = guard.as_ref() else {
            return Vec::new();
        };

        picker
            .fuzzy_search(
                &QueryParser::default().parse(""),
                None,
                FuzzySearchOptions {
                    max_threads: 0,
                    ..Default::default()
                },
            )
            .items
            .iter()
            .map(|item| Line::new(vec![Span::new(item.relative_path(picker))]))
            .collect()
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

        let _input_text_rectangle =
            buffer.draw_border(&input_rectangle, Style::COMMAND_LIST_BORDER);

        let file_list_rectangle =
            buffer.draw_border(&file_list_rectangle, Style::COMMAND_LIST_BORDER);

        buffer.render_lines(self.files(), &file_list_rectangle);
    }

    fn handle_event(&mut self, _event: &Event, _event_context: &mut EventContext) -> EventOutcome {
        EventOutcome::Unhandled
    }

    fn visual_cursor_position(&self) -> Option<Position> {
        None
    }

    fn handle_internal_events(&mut self) -> EventOutcome {
        EventOutcome::Unhandled
    }

    fn kind(&self) -> Option<LayerKind> {
        Some(LayerKind::FilePicker)
    }
}
