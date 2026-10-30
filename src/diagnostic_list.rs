use std::{
    cmp,
    range::Range,
};

use crossterm::event::{
    KeyCode,
    KeyModifiers,
};
use ropey::{
    Rope,
    RopeSlice,
};

use crate::{
    buffer::Buffer,
    cursor::{
        CursorState,
        MovementAction,
        apply_movement,
    },
    document::Diagnostic,
    editor::{
        EditorAction,
        EventContext,
        EventOutcome,
    },
    grapheme_layout::{
        GraphemeLayoutIterator,
        WrapBehavior,
    },
    keymap::KeyBinding,
    style::Style,
    text::{
        ByteIndex,
        RopeSliceExt as _,
        number_width,
    },
    ui::{
        Dimensions,
        Layer,
        LayerEvent,
        LayerKind,
        MaybeBordered,
        NonZeroColumns,
        Position,
        Rectangle,
        Rows,
        Span,
        text_area::TextArea,
    },
};

/// Displays a list of diagnostic information.
pub(crate) struct DiagnosticList {
    text: Rope,
    styles: Vec<DiagnosticStyle>,
    layout: Layout,
    cursor_state: CursorState,
}

impl DiagnosticList {
    pub(crate) fn new(diagnostics: &[&Diagnostic], app_dimensions: Dimensions) -> Self {
        let (text, styles) = diagnostics.iter().fold(
            (String::new(), Vec::new()),
            |(text, mut styles), diagnostic| {
                let message = diagnostic.message().trim_end();

                let start = ByteIndex::new(text.len());
                let range = Range::from(start..start + message.len());

                styles.push(DiagnosticStyle {
                    range,
                    style: Style::diagnostic(diagnostic.severity()),
                });

                (text + message + "\n", styles)
            },
        );

        let text = Rope::from_str(text.trim_suffix('\n'));

        Self {
            layout: Layout::new(text.slice(..), app_dimensions, diagnostics.len()),
            text,
            styles,
            cursor_state: CursorState::default(),
        }
    }

    fn apply_action(&mut self, action: DiagnosticListAction, context: &mut EventContext) {
        match action {
            DiagnosticListAction::Close => {
                context.push_action(EditorAction::RemoveLayer(LayerKind::DiagnosticList));
            }
            DiagnosticListAction::Movement(movement_action) => {
                apply_movement(
                    self.text.slice(..),
                    &mut self.cursor_state,
                    movement_action,
                    None,
                    NonZeroColumns::new(self.layout.text.area().width()),
                );

                self.recalculate_scroll();
            }
        }
    }

    fn style_at(&self, byte_index: ByteIndex) -> Option<&DiagnosticStyle> {
        let partition = self
            .styles
            .partition_point(|entry| entry.range.start <= byte_index);

        let index = partition.checked_sub(1)?;

        self.styles
            .get(index)
            .filter(|style| style.range.contains(&byte_index))
    }

    fn recalculate_scroll(&mut self) {
        self.layout
            .text
            .recalculate_scroll(self.text.slice(..), self.cursor_state.cursor());
    }
}

impl Layer for DiagnosticList {
    fn render(&mut self, buffer: &mut Buffer) {
        buffer.clear_and_style_rectangle(self.layout.container.outer(), Style::COMMAND_LIST);
        buffer.draw_border(&self.layout.container, Style::COMMAND_LIST_BORDER);

        let Some(text_width) = NonZeroColumns::new(self.layout.text.area().width()) else {
            return;
        };

        let text = self.text.slice(..);
        let start_byte = text.line_start_byte(self.layout.text.scroll_offset());

        let mut entries = self
            .styles
            .iter()
            .enumerate()
            .skip(
                self.styles
                    .partition_point(|entry| entry.range.start < start_byte),
            )
            .peekable();

        for grapheme in GraphemeLayoutIterator::new(
            text.slice(start_byte.value()..).graphemes(),
            WrapBehavior::Wrap {
                max_width: text_width,
            },
        ) {
            let row = grapheme.position().top();

            if row >= self.layout.text.area().height() {
                break;
            }

            let byte_index = start_byte + grapheme.byte_index().value();

            if let Some((index, _entry)) =
                entries.next_if(|&(_index, entry)| entry.range.start <= byte_index)
            {
                buffer.render_span(
                    &Span::new(format!("{}. ", index + 1)),
                    &self.layout.gutter.offset().row_offset(row),
                    &self.layout.gutter,
                );
            }

            let style = self
                .style_at(byte_index)
                .map_or_else(Style::new, |entry| entry.style);

            buffer[grapheme.position().offset(self.layout.text.area().offset())]
                .set_content(grapheme.grapheme().display())
                .set_style(style);
        }
    }

    fn handle_event(&mut self, event: &LayerEvent, context: &mut EventContext) -> EventOutcome {
        match *event {
            LayerEvent::Key(key_event) => {
                let key_binding = KeyBinding::from(key_event);

                match (key_binding.code(), key_binding.modifiers()) {
                    (KeyCode::Esc, KeyModifiers::NONE) => {
                        self.apply_action(DiagnosticListAction::Close, context);
                    }
                    (KeyCode::Char('h'), KeyModifiers::NONE) => {
                        self.apply_action(
                            DiagnosticListAction::Movement(MovementAction::MoveLeft),
                            context,
                        );
                    }
                    (KeyCode::Char('j'), KeyModifiers::NONE) => {
                        self.apply_action(
                            DiagnosticListAction::Movement(MovementAction::MoveDown),
                            context,
                        );
                    }
                    (KeyCode::Char('k'), KeyModifiers::NONE) => {
                        self.apply_action(
                            DiagnosticListAction::Movement(MovementAction::MoveUp),
                            context,
                        );
                    }
                    (KeyCode::Char('l'), KeyModifiers::NONE) => {
                        self.apply_action(
                            DiagnosticListAction::Movement(MovementAction::MoveRight),
                            context,
                        );
                    }
                    _ => {}
                }

                // we swallow all key events even if we don't match anything in
                // order to prevent interactions with other
                // layers; diagnostics list is "focused" until
                // we close it
                EventOutcome::Handled
            }

            LayerEvent::FocusGained
            | LayerEvent::FocusLost
            | LayerEvent::Mouse(_)
            | LayerEvent::Paste => EventOutcome::Unhandled,
        }
    }

    fn visual_cursor_position(&self) -> Option<Position> {
        Some(
            self.layout
                .text
                .visual_cursor_position(self.text.slice(..), self.cursor_state.cursor()),
        )
    }

    fn handle_internal_events(&mut self) -> EventOutcome {
        EventOutcome::Unhandled
    }

    fn kind(&self) -> Option<LayerKind> {
        Some(LayerKind::DiagnosticList)
    }

    fn resize(&mut self, app_dimensions: Dimensions) {
        self.layout
            .resize(self.text.slice(..), app_dimensions, self.styles.len());
        self.recalculate_scroll();
    }
}

#[derive(Debug, Clone, Copy)]
enum DiagnosticListAction {
    Close,
    Movement(MovementAction),
}

#[derive(Debug)]
struct DiagnosticStyle {
    range: Range<ByteIndex>,
    style: Style,
}

#[derive(Debug)]
struct Layout {
    /// The entire diagnostic list.
    container: MaybeBordered,
    /// Gutter where diagnostics are numbered.
    gutter: Rectangle,
    /// Diagnostic messages.
    text: TextArea,
}

impl Layout {
    const MIN_HEIGHT: Rows = Rows::new(5);

    fn new(text: RopeSlice<'_>, app_dimensions: Dimensions, diagnostics_count: usize) -> Self {
        let app_rectangle = Rectangle::from_dimensions(app_dimensions);

        let max_height = app_rectangle.height();

        let width = app_rectangle.width() * 8 / 10;

        let (content_width, border_height) =
            Rectangle::from_dimensions(Dimensions::new(width, max_height))
                .clip_border()
                .map_or((width, Rows::new(0)), |rectangle| {
                    (rectangle.width(), Rows::new(2))
                });

        let gutter_width = number_width(diagnostics_count) + ". ".len();

        let content_height = NonZeroColumns::new(content_width.saturating_sub(gutter_width))
            .map_or(Rows::new(0), |text_width| {
                GraphemeLayoutIterator::new(text.graphemes(), WrapBehavior::Wrap {
                    max_width: text_width,
                })
                .last()
                .map_or(Rows::new(1), |grapheme| {
                    grapheme.end_position().top() + Rows::new(1)
                })
            });

        let desired_height = content_height + border_height;

        let height = cmp::min(max_height, cmp::max(Self::MIN_HEIGHT, desired_height));

        let container = app_rectangle.at_center(Dimensions::new(width, height));

        let container = MaybeBordered::new(container);

        assert_eq!(
            container.inner().width(),
            content_width,
            "`content_width` must have been calculated correctly"
        );

        let (gutter_rectangle, text_rectangle) = container.inner().split_at_column(gutter_width);

        Self {
            container,
            gutter: gutter_rectangle,
            text: TextArea::new(text_rectangle),
        }
    }

    fn resize(
        &mut self,
        text: RopeSlice<'_>,
        app_dimensions: Dimensions,
        diagnostics_count: usize,
    ) {
        let layout = Self::new(text, app_dimensions, diagnostics_count);

        *self = Self {
            text: self.text.with_area(layout.text.into_area()),
            ..layout
        };
    }
}
