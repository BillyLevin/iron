use std::cmp;

use crossterm::event::{
    Event,
    KeyCode,
    KeyModifiers,
};

use crate::{
    buffer::{
        Buffer,
        DrawBorderOutcome,
    },
    document::DiagnosticMessage,
    editor::{
        EditorAction,
        EventContext,
        EventOutcome,
    },
    keymap::KeyBinding,
    style::Style,
    text::number_width,
    ui::{
        Dimensions,
        Layer,
        LayerKind,
        Position,
        Rectangle,
        Rows,
        WrappedLineLayout,
    },
};

/// Displays a list of diagnostic information.
pub(crate) struct DiagnosticList {
    diagnostics: Vec<DiagnosticMessage>,
}

impl DiagnosticList {
    pub(crate) const fn new(diagnostics: Vec<DiagnosticMessage>) -> Self {
        Self { diagnostics }
    }

    fn apply_action(action: DiagnosticListAction, context: &mut EventContext) {
        match action {
            DiagnosticListAction::Close => {
                context.push_action(EditorAction::RemoveLayer(LayerKind::DiagnosticList));
            }
        }
    }
}

impl Layer for DiagnosticList {
    fn render(&mut self, buffer: &mut Buffer) {
        let diagnostics_count = self.diagnostics.len();
        let max_number_width = number_width(diagnostics_count);

        let app_rectangle = Rectangle::from_dimensions(buffer.dimensions());

        let max_height = app_rectangle.height();
        let min_height = Rows::new(5);

        let width = app_rectangle.width() * 8 / 10;
        let (content_width, border_height) =
            Rectangle::from_dimensions(Dimensions::new(width, max_height))
                .clip_border()
                .map_or((width, Rows::new(0)), |rectangle| {
                    (rectangle.width(), Rows::new(2))
                });

        let lines = self
            .diagnostics
            .iter()
            .enumerate()
            .map(|(index, diagnostic)| {
                let line_number = index + 1;
                let padding = max_number_width - number_width(line_number);

                let text = format!("{line_number}.{:padding$} ", "", padding = padding.value())
                    + diagnostic.message();

                (text, Style::diagnostic(diagnostic.severity()))
            });

        let line_layout = WrappedLineLayout::new(lines, content_width);

        let desired_height = line_layout.height() + border_height;

        let height = cmp::min(max_height, cmp::max(min_height, desired_height));

        let rectangle = app_rectangle.at_center(Dimensions::new(width, height));

        buffer.clear_and_style_rectangle(&rectangle, Style::COMMAND_LIST);
        let rectangle = match buffer.draw_border(rectangle, Style::COMMAND_LIST_BORDER) {
            DrawBorderOutcome::Drawn { inner_rectangle } => inner_rectangle,
            DrawBorderOutcome::NotDrawn { original_rectangle } => original_rectangle,
        };

        assert_eq!(
            rectangle.width(),
            content_width,
            "`content_width` must have been calculated correctly"
        );

        buffer.render_lines(line_layout.lines(), &rectangle);
    }

    fn handle_event(
        &mut self,
        event: &crossterm::event::Event,
        context: &mut EventContext,
    ) -> EventOutcome {
        match *event {
            Event::Key(key_event) => {
                let key_binding = KeyBinding::from(key_event);

                match (key_binding.code(), key_binding.modifiers()) {
                    (KeyCode::Esc, KeyModifiers::NONE) => {
                        Self::apply_action(DiagnosticListAction::Close, context);
                    }
                    _ => {}
                }

                // we swallow all key events even if we don't match anything in
                // order to prevent interactions with other
                // layers; diagnostics list is "focused" until
                // we close it
                EventOutcome::Handled
            }

            Event::FocusGained
            | Event::FocusLost
            | Event::Mouse(_)
            | Event::Paste(_)
            | Event::Resize(_, _) => EventOutcome::Unhandled,
        }
    }

    fn visual_cursor_position(&self) -> Option<Position> {
        None
    }

    fn handle_internal_events(&mut self) -> EventOutcome {
        EventOutcome::Unhandled
    }

    fn kind(&self) -> Option<LayerKind> {
        Some(LayerKind::DiagnosticList)
    }
}

#[derive(Debug, Clone, Copy)]
enum DiagnosticListAction {
    Close,
}
