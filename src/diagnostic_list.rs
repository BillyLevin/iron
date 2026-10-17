use std::{
    cmp,
    range::Range,
};

use crossterm::event::{
    Event,
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
        Selection,
        move_cursor_down,
        move_cursor_left,
        move_cursor_right,
        move_cursor_up,
        visual_cursor_position,
    },
    document::DiagnosticMessage,
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
        LineIndex,
        RopeSliceExt as _,
        number_width,
    },
    ui::{
        Columns,
        Dimensions,
        Layer,
        LayerKind,
        NonZeroColumns,
        Position,
        Rectangle,
        Rows,
        Span,
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
    pub(crate) fn new(diagnostics: &[DiagnosticMessage], app_dimensions: Dimensions) -> Self {
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
            DiagnosticListAction::MoveLeft => {
                self.cursor_state.set_selection(move_cursor_left(
                    self.text.slice(..),
                    Selection::default().with_cursor(self.cursor_state.cursor()),
                    1,
                ));
            }
            DiagnosticListAction::MoveDown => {
                let Some(text_width) = NonZeroColumns::new(self.layout.text.width()) else {
                    return;
                };

                self.cursor_state.set_selection(move_cursor_down(
                    self.text.slice(..),
                    Selection::default().with_cursor(self.cursor_state.cursor()),
                    // TODO: pass in desired column when it's implemented
                    Columns::new(0),
                    text_width,
                    1,
                ));
            }
            DiagnosticListAction::MoveUp => {
                let Some(text_width) = NonZeroColumns::new(self.layout.text.width()) else {
                    return;
                };

                self.cursor_state.set_selection(move_cursor_up(
                    self.text.slice(..),
                    Selection::default().with_cursor(self.cursor_state.cursor()),
                    // TODO: pass in desired column when it's implemented
                    Columns::new(0),
                    text_width,
                    1,
                ));
            }
            DiagnosticListAction::MoveRight => {
                self.cursor_state.set_selection(move_cursor_right(
                    self.text.slice(..),
                    Selection::default().with_cursor(self.cursor_state.cursor()),
                    1,
                ));
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

    fn resize(&mut self, dimensions: Dimensions) {
        self.layout = Layout::new(self.text.slice(..), dimensions, self.styles.len());
    }
}

impl Layer for DiagnosticList {
    fn render(&mut self, buffer: &mut Buffer) {
        buffer.clear_and_style_rectangle(&self.layout.container, Style::COMMAND_LIST);
        buffer.draw_border(self.layout.container.clone(), Style::COMMAND_LIST_BORDER);

        let Some(text_width) = NonZeroColumns::new(self.layout.text.width()) else {
            return;
        };

        for (index, entry) in self.styles.iter().enumerate() {
            let row = visual_cursor_position(
                self.text.slice(..),
                entry.range.start,
                LineIndex::new(0),
                text_width,
            )
            .top();

            buffer.render_span(
                &Span::new(format!("{}. ", index + 1)),
                &self.layout.gutter.offset().row_offset(row),
                &self.layout.gutter,
            );
        }

        for grapheme in
            GraphemeLayoutIterator::new(self.text.slice(..).graphemes(), WrapBehavior::Wrap {
                max_width: text_width,
            })
        {
            if grapheme.position().top() >= self.layout.text.height() {
                break;
            }

            let style = self
                .style_at(grapheme.byte_index())
                .map_or_else(Style::new, |entry| entry.style);

            buffer[grapheme.position().offset(self.layout.text.offset())]
                .set_content(grapheme.grapheme().as_str())
                .set_style(style);
        }
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
                        self.apply_action(DiagnosticListAction::Close, context);
                    }
                    (KeyCode::Char('h'), KeyModifiers::NONE) => {
                        self.apply_action(DiagnosticListAction::MoveLeft, context);
                    }
                    (KeyCode::Char('j'), KeyModifiers::NONE) => {
                        self.apply_action(DiagnosticListAction::MoveDown, context);
                    }
                    (KeyCode::Char('k'), KeyModifiers::NONE) => {
                        self.apply_action(DiagnosticListAction::MoveUp, context);
                    }
                    (KeyCode::Char('l'), KeyModifiers::NONE) => {
                        self.apply_action(DiagnosticListAction::MoveRight, context);
                    }
                    _ => {}
                }

                // we swallow all key events even if we don't match anything in
                // order to prevent interactions with other
                // layers; diagnostics list is "focused" until
                // we close it
                EventOutcome::Handled
            }

            Event::Resize(columns, rows) => {
                self.resize(Dimensions::new(Columns::from(columns), Rows::from(rows)));

                // TODO: this needs to not be `Handled` because the document
                // needs to resize too. should add another variant to
                // distinguish between bubbling and capturing events
                EventOutcome::Unhandled
            }

            Event::FocusGained | Event::FocusLost | Event::Mouse(_) | Event::Paste(_) => {
                EventOutcome::Unhandled
            }
        }
    }

    fn visual_cursor_position(&self) -> Option<Position> {
        Some(
            visual_cursor_position(
                self.text.slice(..),
                self.cursor_state.cursor(),
                // TODO: pass in scroll offset here when implementing scrolling
                LineIndex::new(0),
                NonZeroColumns::new(self.layout.text.width())?,
            )
            .offset(self.layout.text.offset()),
        )
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
    MoveLeft,
    MoveDown,
    MoveUp,
    MoveRight,
}

#[derive(Debug)]
struct DiagnosticStyle {
    range: Range<ByteIndex>,
    style: Style,
}

#[derive(Debug)]
struct Layout {
    /// The entire diagnostic list.
    container: Rectangle,
    /// Gutter where diagnostics are numbered.
    gutter: Rectangle,
    /// Diagnostic messages.
    text: Rectangle,
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

        let outer_rectangle = app_rectangle.at_center(Dimensions::new(width, height));

        let inner_rectangle = outer_rectangle
            .clip_border()
            .unwrap_or_else(|| outer_rectangle.clone());

        assert_eq!(
            inner_rectangle.width(),
            content_width,
            "`content_width` must have been calculated correctly"
        );

        let (gutter_rectangle, text_rectangle) = inner_rectangle.split_at_column(gutter_width);

        Self {
            container: outer_rectangle,
            gutter: gutter_rectangle,
            text: text_rectangle,
        }
    }
}
