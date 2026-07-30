use std::{
    cmp,
    fmt,
    fs::File,
    io::{
        BufReader,
        BufWriter,
    },
    iter,
    mem,
    num::{
        NonZero,
        NonZeroUsize,
    },
    ops::{
        ControlFlow,
        Range,
    },
    path::PathBuf,
    sync::{
        self,
        mpsc::{
            Receiver,
            Sender,
        },
    },
    thread,
};

use anyhow::Context as _;
use crossterm::event::{
    Event,
    KeyCode,
    KeyEvent,
    KeyModifiers,
};
use gen_lsp_types::{
    DiagnosticSeverity,
    PublishDiagnosticsParams,
};
use itertools::Itertools as _;
use ropey::{
    LineType,
    Rope,
    RopeSlice,
};
use unicode_segmentation::UnicodeSegmentation as _;
use url::Url;

use crate::{
    buffer::Buffer,
    editor::{
        EditorAction,
        EventContext,
        EventOutcome,
    },
    grapheme_layout::{
        GraphemeLayoutIterator,
        WrapBehavior,
    },
    highlight::{
        Checkpoint,
        Highlighter,
        Token,
    },
    jujutsu::JJInfo,
    keymap::{
        BehaviorAction,
        DocumentAction,
        EditAction,
        KeyBinding,
        KeyMap,
        KeySequence,
        MovementAction,
    },
    language::Language,
    lsp::{
        DocumentLspId,
        DocumentSnapshot,
        DocumentVersion,
        LspTextEdit,
        PositionEncoding,
    },
    style::Style,
    text::{
        ByteIndex,
        LeftChar,
        LineIndex,
        RightChar,
        RopeSliceExt as _,
        TAB_VISUAL_WIDTH,
        VisualLineInfo,
        text_width,
    },
    ui::{
        Alignment,
        Columns,
        Dimensions,
        Layer,
        LayerKind,
        NonZeroColumns,
        Position,
        Rectangle,
        Rows,
        Span,
        WrapOutcome,
        spans_width,
    },
};

#[derive(Debug)]
pub(crate) struct Document {
    text: Rope,
    selection: Selection,

    normal_keymap: KeyMap,
    insert_keymap: KeyMap,
    visual_keymap: KeyMap,

    /// Number of lines from the top of the file that the buffer text should
    /// start from.
    scroll_offset: LineIndex,

    /// When navigating vertically, the cursor will be moved to the left if the
    /// next line is narrower than the current. We use this field to track
    /// where the cursor would ideally be so that we can move it there if
    /// the line is wide enough.
    ///
    /// The value is relative to the start of the **text**, and does NOT include
    /// the `gutter_width`.
    desired_cursor_column: Option<Columns>,

    /// The keys that have been pressed which may add up to a registered
    /// keybinding. Used in the `KeyMap` lookups.
    key_sequence: KeySequence,

    file_path: PathBuf,

    lsp_id: DocumentLspId,

    layout_info: LayoutInfo,

    /// The detected language of the document - this is based on the extension
    /// rather than the contents of the file.
    language: Language,

    jj_info: JJInfo,

    error: Option<String>,

    highlights: HighlightCache,

    highlight_request_tx: Sender<HighlightRequest>,
    highlight_response_rx: Receiver<HighlightCache>,

    version: DocumentVersion,

    diagnostics: Vec<Diagnostic>,
}

impl Document {
    pub(crate) fn new(file_path: PathBuf, dimensions: Dimensions) -> anyhow::Result<Self> {
        let text = Rope::from_reader(BufReader::new(File::open(&file_path)?))?;

        let language = Language::new(&file_path);
        let lsp_id = DocumentLspId::new(&file_path, language)?;

        let (highlight_request_tx, highlight_response_rx) = spawn_highlights_thread();

        let this = Self {
            text,
            selection: Selection::default(),
            normal_keymap: KeyMap::normal(),
            insert_keymap: KeyMap::insert(),
            visual_keymap: KeyMap::visual(),
            scroll_offset: LineIndex::default(),
            desired_cursor_column: None,
            key_sequence: KeySequence::new(Mode::Normal),
            language,
            file_path,
            lsp_id,
            layout_info: LayoutInfo::new(dimensions),
            jj_info: JJInfo::default(),
            error: None,
            highlights: HighlightCache::new(),
            highlight_request_tx,
            highlight_response_rx,
            version: DocumentVersion::default(),
            diagnostics: Vec::new(),
        };

        this.request_highlight_refresh();

        Ok(this)
    }

    pub(crate) fn handle_key_event(
        &mut self,
        key_event: KeyEvent,
        context: &mut EventContext,
    ) -> EventOutcome {
        let keymap = match self.mode() {
            Mode::Normal => &self.normal_keymap,
            Mode::Insert => &self.insert_keymap,
            Mode::Visual => &self.visual_keymap,
        };

        self.key_sequence.push(KeyBinding::from(key_event));

        let (keys, count) = self.key_sequence.parse();

        let maybe_action = match keymap.get(&keys) {
            Some(&KeyMap::BindingPart { .. }) => {
                // the key sequence could form a binding with subsequent key events. since
                // we'd already pushed the latest event to the sequence store, we are done
                None
            }
            Some(&KeyMap::Action(action)) => Some(action),
            None => {
                // the current key sequence does not form a binding for any of the registered
                // commands. therefore, we clear the sequence
                self.key_sequence.clear();

                // look for any fallback events that aren't registered in the map
                self.event_fallback(key_event)
            }
        };

        if let Some(action) = maybe_action {
            self.apply_action(action, count, context);

            self.key_sequence.clear();

            self.clamp_cursor();
            self.recalculate_scroll();
        }

        EventOutcome::Handled
    }

    pub(crate) fn resize(&mut self, dimensions: Dimensions) {
        self.layout_info = LayoutInfo::new(dimensions);
        self.recalculate_scroll();
    }

    /// Displays the keybindings (if any) that are currently possible for the
    /// user to invoke, based on the current sequence of key events.
    fn render_key_hint(&self, buffer: &mut Buffer) {
        // TODO: would be nice to display the count in the labels where it's relevant
        let (keys, _count) = self.key_sequence.parse();

        if keys.is_empty() {
            return;
        }

        let keymap = match self.mode() {
            Mode::Normal => &self.normal_keymap,
            Mode::Insert => &self.insert_keymap,
            Mode::Visual => &self.visual_keymap,
        };

        let Some(&KeyMap::BindingPart { ref map }) = keymap.get(&keys) else {
            return;
        };

        let (hints, max_width, max_height) = map.iter().fold(
            (String::new(), Columns::new(0), Rows::new(0)),
            |(hints, max_width, max_height), (event, map_node)| {
                let key = event.to_string();
                let label = match *map_node {
                    KeyMap::BindingPart { .. } => "...",
                    KeyMap::Action(action) => action.label(),
                };

                let hint = format!("{key}: {label}\n");

                (
                    hints + &hint,
                    cmp::max(max_width, text_width(&hint)),
                    // we will disable wrapping so we can simply increment the count
                    max_height + Rows::new(1),
                )
            },
        );

        let container = &self.layout_info.content_rect;

        let hints_rectangle = container.at_bottom_right(Dimensions::new(
            cmp::min(container.width(), max_width),
            cmp::min(container.height(), max_height),
        ));

        buffer.clear_and_style_rectangle(&hints_rectangle, Style::HINTS);

        for visual_grapheme in
            GraphemeLayoutIterator::new(hints.graphemes(true), WrapBehavior::NoWrap)
        {
            if visual_grapheme.position().top() >= hints_rectangle.height() {
                break;
            }

            // wrapping has been turned off, and therefore we ignore all graphemes
            // that would overflow
            if visual_grapheme.position().left() >= hints_rectangle.width() {
                continue;
            }

            buffer[visual_grapheme.position().offset(hints_rectangle.offset())]
                .set_content(visual_grapheme.grapheme().as_str());
        }
    }

    fn render_status_line(&self, buffer: &mut Buffer) {
        let (status_rect, message_rect) =
            self.layout_info.status_line_rect.split_at_row(Rows::new(1));

        buffer.clear_and_style_rectangle(&status_rect, Style::STATUS_LINE);
        buffer.clear_and_style_rectangle(&message_rect, Style::STATUS_LINE_MESSAGES);

        let mode_span = Span::new(format!(" {} ", self.mode())).with_style(Style::STATUS_LINE_MODE);

        let file_name_span = self
            .file_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| Span::new(format!(" {name} ")).with_style(Style::STATUS_LINE_TEXT));

        let jj_desc_span = self
            .jj_info
            .description()
            .map(|desc| Span::new(format!(r#" "{desc}" "#)).with_style(Style::STATUS_LINE_TEXT));

        let (jj_diff_added_span, jj_diff_removed_span) = self
            .jj_info
            .diff()
            .map(|diff| {
                (
                    Span::new(format!(" +{}", diff.added())).with_style(Style::DIFF_ADDED),
                    Span::new(format!(" -{} ", diff.removed())).with_style(Style::DIFF_REMOVED),
                )
            })
            .unzip();

        let left_spans: Vec<Span> = iter::once(Some(mode_span))
            .chain([
                file_name_span,
                jj_diff_added_span,
                jj_diff_removed_span,
                jj_desc_span,
            ])
            .flatten()
            .collect();

        let language_span =
            Span::new(format!(" {} ", self.language)).with_style(Style::STATUS_LINE_TEXT);

        let right_spans = match self.key_sequence.to_string().as_str() {
            "" => vec![language_span],
            keys => {
                vec![
                    Span::new(format!(" {keys} ")).with_style(Style::STATUS_LINE_TEXT),
                    language_span,
                ]
            }
        };

        let (left_rect, right_rect) = status_rect.split_at_column(spans_width(&left_spans));

        buffer.render_spans(&left_spans, &left_rect, Alignment::Left);
        buffer.render_spans(&right_spans, &right_rect, Alignment::Right);

        if let Some(ref error) = self.error {
            buffer.render_span(
                &Span::new(error).with_style(Style::STATUS_LINE_ERROR),
                &message_rect.offset(),
                &message_rect,
            );
        }
    }

    const fn set_cursor(&mut self, index: ByteIndex) {
        self.selection.cursor = index;
    }

    const fn set_anchor(&mut self, index: ByteIndex) {
        self.selection.anchor = index;
    }

    fn apply_action(
        &mut self,
        action: DocumentAction,
        count: Option<NonZeroUsize>,
        event_context: &mut EventContext,
    ) {
        puffin::profile_function!();

        let action_count = count.map_or(1, NonZero::get);

        match action {
            DocumentAction::Movement(MovementAction::MoveDown) => {
                self.move_cursor_down(action_count);
            }
            DocumentAction::Movement(MovementAction::MoveUp) => self.move_cursor_up(action_count),
            DocumentAction::Movement(MovementAction::MoveRight) => {
                self.move_cursor_right(action_count);
            }
            DocumentAction::Movement(MovementAction::MoveLeft) => {
                self.move_cursor_left(action_count);
            }
            DocumentAction::Movement(MovementAction::MoveNextWordStart) => {
                self.move_cursor_next_word_start(action_count);
            }
            DocumentAction::Movement(MovementAction::MovePrevWordStart) => {
                self.move_cursor_prev_word_start(action_count);
            }
            DocumentAction::Behavior(BehaviorAction::SwitchToInsertMode) => self.insert_mode(),
            DocumentAction::Behavior(BehaviorAction::SwitchToNormalMode) => self.normal_mode(),
            DocumentAction::Behavior(BehaviorAction::SwitchToVisualMode) => self.visual_mode(),
            DocumentAction::Movement(MovementAction::MoveLineEnd) => self.move_cursor_line_end(),
            DocumentAction::Movement(MovementAction::MoveLineStart) => {
                self.move_cursor_line_start();
            }
            DocumentAction::Movement(MovementAction::MoveLineFirstNonBlank) => {
                self.move_cursor_first_non_blank();
            }
            DocumentAction::Movement(MovementAction::MoveNextParagraph) => {
                self.move_cursor_next_paragraph(action_count);
            }
            DocumentAction::Movement(MovementAction::MovePrevParagraph) => {
                self.move_cursor_prev_paragraph(action_count);
            }
            DocumentAction::Movement(MovementAction::GoToLastLine) => self.go_to_last_line(),
            DocumentAction::Movement(MovementAction::GoToNthOrLastLine) => {
                self.go_to_nth_or_last_line(count);
            }
            DocumentAction::Movement(MovementAction::GoToNthOrFirstLine) => {
                self.go_to_nth_or_first_line(count);
            }
            DocumentAction::Movement(MovementAction::MoveWordEnd) => {
                self.move_cursor_word_end(action_count);
            }
            DocumentAction::Movement(MovementAction::ReverseSelection) => self.reverse_selection(),
            DocumentAction::Movement(MovementAction::SelectCurrentWord) => {
                self.select_current_word();
            }
            DocumentAction::Behavior(BehaviorAction::OpenCommandList) => {
                Self::open_command_list(event_context);
            }
            DocumentAction::Behavior(BehaviorAction::ClearInput) => self.clear_input(),
            DocumentAction::Movement(MovementAction::VerticallyCenter) => {
                self.center_cursor_vertically();
            }
            DocumentAction::Behavior(BehaviorAction::OpenFilePicker) => {
                Self::open_file_picker(event_context);
            }
            DocumentAction::Movement(MovementAction::GoToPairMatch) => {
                self.go_to_pair_match();
            }
            DocumentAction::Behavior(BehaviorAction::AppendText) => self.append_text(),

            DocumentAction::Edit(edit_action) => {
                self.handle_edit_action(edit_action, action_count, event_context);
            }
        }

        if action.should_reset_desired_column() {
            self.clear_desired_column();
        }
    }

    fn handle_edit_action(&mut self, action: EditAction, count: usize, context: &mut EventContext) {
        let (transaction, mode) = match action {
            EditAction::InsertChar(ch) => (self.insert_char(ch), None),
            EditAction::DeleteGrapheme => (self.delete_grapheme(), None),
            EditAction::InsertNewline => (self.insert_newline(), None),
            EditAction::DeleteWord => (self.delete_word(count), None),
            EditAction::ChangeWord => (self.delete_word(count), Some(Mode::Insert)),
            EditAction::DeleteToLineEnd => (self.delete_to_line_end(), None),
            EditAction::ChangeToLineEnd => (self.delete_to_line_end(), Some(Mode::Insert)),
            EditAction::DeleteToLineStart => (self.delete_to_line_start(), None),
            EditAction::DeleteToLineFirstNonBlank => (self.delete_to_first_non_blank(), None),
            EditAction::DeleteLine => (self.delete_line(count), None),
            EditAction::DeleteWholeWord => (self.delete_whole_word(), None),
            EditAction::DeleteToPrevWordStart => (self.delete_to_prev_word_start(count), None),
            EditAction::AppendTextLineEnd => (self.append_text_line_end(), Some(Mode::Insert)),
            EditAction::DeleteToWordEnd => (self.delete_to_word_end(count), None),
            EditAction::ChangeToLineStart => (self.delete_to_line_start(), Some(Mode::Insert)),
            EditAction::ChangeToLineFirstNonBlank => {
                (self.delete_to_first_non_blank(), Some(Mode::Insert))
            }
            EditAction::ChangeLine => (self.change_line(count), Some(Mode::Insert)),
            EditAction::ChangeWholeWord => (self.delete_whole_word(), Some(Mode::Insert)),
            EditAction::ChangeToPrevWordStart => {
                (self.delete_to_prev_word_start(count), Some(Mode::Insert))
            }
            EditAction::ChangeToWordEnd => (self.delete_to_word_end(count), Some(Mode::Insert)),
            EditAction::DeleteSelection => (self.delete_selection(), Some(Mode::Normal)),
            EditAction::ChangeSelection => (self.delete_selection(), Some(Mode::Insert)),
            EditAction::OpenLineBelow => (self.open_new_line_below(), Some(Mode::Insert)),
            EditAction::OpenLineAbove => (self.open_new_line_above(), Some(Mode::Insert)),
            EditAction::DeleteDown => (self.delete_down(count), None),
            EditAction::DeleteUp => (self.delete_up(count), None),
            EditAction::InsertTab => (self.insert_tab(), None),
        };

        self.apply_transaction(transaction, context);

        match mode {
            Some(Mode::Normal) => self.normal_mode(),
            Some(Mode::Insert) => self.insert_mode(),
            Some(Mode::Visual) => self.visual_mode(),
            None => {}
        }
    }

    fn apply_transaction(&mut self, transaction: Transaction, context: &mut EventContext) {
        if let Some(edit) = transaction.edit
            && !edit.is_noop()
        {
            let initial_text = self.text.clone();
            self.apply_edit(&edit);
            self.on_edit(edit, initial_text, context);
        }

        self.selection = transaction.selection;
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "this is the only place we can use the rope-mutating methods"
    )]
    fn apply_edit(&mut self, edit: &TextEdit) {
        self.text
            .remove(edit.range.start.value()..edit.range.end.value());
        self.text
            .insert(edit.range.start.value(), &edit.replacement);
    }

    fn on_edit(&mut self, edit: TextEdit, initial_text: Rope, context: &mut EventContext) {
        self.diagnostics.clear();
        self.version = self.version.next();
        self.request_highlight_refresh();

        context.push_action(EditorAction::DocumentChanged(LspTextEdit::new(
            initial_text,
            edit,
        )));
    }

    fn move_cursor_down(&mut self, count: usize) {
        let Some(text_width) = self.content_layout().max_text_width() else {
            return;
        };

        for _ in 0..count {
            let target_column = self.desired_column(text_width);
            let text = self.text.slice(..);

            let byte = VisualLineInfo::new(
                &self.text,
                text.line_idx_containing_byte(self.selection.cursor),
                text_width,
            )
            .next_at_column(self.selection.cursor, target_column);

            if let Some(byte_index) = byte {
                self.set_cursor(byte_index);
            }
        }
    }

    fn move_cursor_up(&mut self, count: usize) {
        let Some(text_width) = self.content_layout().max_text_width() else {
            return;
        };

        for _ in 0..count {
            let target_column = self.desired_column(text_width);

            let text = self.text.slice(..);

            let byte = VisualLineInfo::new(
                &self.text,
                text.line_idx_containing_byte(self.selection.cursor),
                text_width,
            )
            .prev_at_column(self.selection.cursor, target_column);

            if let Some(byte_index) = byte {
                self.set_cursor(byte_index);
            }
        }
    }

    fn move_cursor_right(&mut self, count: usize) {
        for _ in 0..count {
            self.set_cursor(
                self.text
                    .slice(..)
                    .next_grapheme_position(self.selection.cursor),
            );
        }
    }

    fn move_cursor_left(&mut self, count: usize) {
        for _ in 0..count {
            self.set_cursor(
                self.text
                    .slice(..)
                    .previous_grapheme_position(self.selection.cursor),
            );
        }
    }

    fn move_cursor_next_word_start(&mut self, count: usize) {
        for _ in 0..count {
            let byte_index = match self
                .text
                .slice(self.selection.cursor.value()..)
                .chars()
                .tuple_windows()
                .map(|(left, right)| (LeftChar::new(left), RightChar::new(right)))
                .try_fold(self.selection.cursor, |index, (left, right)| {
                    let next_index = index + left.ch().len_utf8();

                    if right.is_word_start(left) {
                        ControlFlow::Break(next_index)
                    } else {
                        ControlFlow::Continue(next_index)
                    }
                }) {
                ControlFlow::Continue(index) | ControlFlow::Break(index) => index,
            };

            self.set_cursor(byte_index);
        }
    }

    fn move_cursor_prev_word_start(&mut self, count: usize) {
        for _ in 0..count {
            let byte_index = self
                .text
                .slice(..self.selection.cursor.value())
                .chars_at(self.selection.cursor.value())
                .reversed()
                .tuple_windows()
                .map(|(right, left)| (LeftChar::new(left), RightChar::new(right)))
                .try_fold(self.selection.cursor, |index, (left, right)| {
                    let next_index = index.saturating_sub(left.ch().len_utf8());

                    if right.is_word_start(left) {
                        ControlFlow::Break(next_index)
                    } else {
                        ControlFlow::Continue(next_index)
                    }
                })
                .break_value()
                .unwrap_or(ByteIndex::new(0));

            self.set_cursor(byte_index);
        }
    }

    /// Ensures that the cursor does not go past the end of the file.
    fn clamp_cursor(&mut self) {
        self.set_cursor(cmp::min(
            self.selection.cursor,
            ByteIndex::new(self.text.slice(..).len().saturating_sub(1)),
        ));
    }

    fn recalculate_scroll(&mut self) {
        let text = self.text.slice(..);
        let cursor_line = text.line_idx_containing_byte(self.selection.cursor);

        let height = self.layout_info.content_rect.height();

        if cursor_line < self.scroll_offset {
            // upwards scroll
            self.scroll_offset = cursor_line;
        } else {
            // move the scroll offset further down so that we don't have to scan as much of
            // the rope when calculating the cursor position. `scroll_offset` will still be
            // off the screen, so it won't impact the calculation
            if cursor_line > self.scroll_offset + LineIndex::new(height.value()) {
                self.scroll_offset = cursor_line.saturating_sub(height.value());
            }

            let cursor = self.visual_cursor_position_impl();

            // downwards scroll
            if cursor.top() >= height {
                self.scroll_offset += LineIndex::from(cursor.top().value() - height.value() + 1);
            }
        }

        self.scroll_offset = cmp::min(cursor_line, self.scroll_offset);
    }

    fn center_cursor_vertically(&mut self) {
        let text = self.text.slice(..);
        let cursor_line = text.line_idx_containing_byte(self.selection.cursor);
        let middle = self.layout_info.content_rect.height() / 2;

        self.scroll_offset = cursor_line.saturating_sub(middle);
    }

    /// Gets (or inserts the current cursor column) the desired column to
    /// navigate to on vertical cursor movement.
    fn desired_column(&mut self, max_width: NonZeroColumns) -> Columns {
        *self.desired_cursor_column.get_or_insert_with(|| {
            let text = self.text.slice(..);

            let line_start =
                text.line_start_byte(text.line_idx_containing_byte(self.selection.cursor));

            text.slice(line_start.value()..self.selection.cursor.value())
                .chunks()
                .map(text_width)
                .sum::<Columns>()
                .map(|cols| cols % max_width.get().value())
        })
    }

    const fn clear_desired_column(&mut self) {
        self.desired_cursor_column = None;
    }

    const fn event_fallback(&self, key_event: KeyEvent) -> Option<DocumentAction> {
        match self.mode() {
            Mode::Normal | Mode::Visual => None,
            Mode::Insert => {
                if let KeyCode::Char(ch) = key_event.code
                    && !key_event.modifiers.contains(KeyModifiers::CONTROL)
                    && !key_event.modifiers.contains(KeyModifiers::ALT)
                {
                    Some(DocumentAction::Edit(EditAction::InsertChar(ch)))
                } else {
                    None
                }
            }
        }
    }

    const fn insert_mode(&mut self) {
        self.key_sequence.set_mode(Mode::Insert);
    }

    const fn normal_mode(&mut self) {
        self.key_sequence.set_mode(Mode::Normal);
    }

    const fn visual_mode(&mut self) {
        self.selection.anchor = self.selection.cursor;
        self.key_sequence.set_mode(Mode::Visual);
    }

    fn insert_char(&self, ch: char) -> Transaction {
        Transaction::new(
            Some(TextEdit::insert(self.selection.cursor, ch)),
            self.selection
                .with_cursor(self.selection.cursor + ch.len_utf8()),
        )
    }

    fn delete_grapheme(&self) -> Transaction {
        let start = self
            .text
            .slice(..)
            .previous_grapheme_position(self.selection.cursor);

        Transaction::new(
            Some(TextEdit::delete(start..self.selection.cursor)),
            self.selection.with_cursor(start),
        )
    }

    fn insert_newline(&self) -> Transaction {
        self.insert_char('\n')
    }

    /// Moves to the cursor to the last non-linebreak grapheme on the current
    /// line.
    fn move_cursor_line_end(&mut self) {
        let text = self.text.slice(..);
        let line_index = text.line_idx_containing_byte(self.selection.cursor);

        self.set_cursor(cmp::max(
            text.line_start_byte(line_index),
            text.previous_grapheme_position(text.line_break(line_index).position),
        ));
    }

    fn move_cursor_line_start(&mut self) {
        let text = self.text.slice(..);

        self.set_cursor(text.line_start_byte(text.line_idx_containing_byte(self.selection.cursor)));
    }

    /// Moves the cursor to the first non-whitespace character on the current
    /// line.
    fn move_cursor_first_non_blank(&mut self) {
        let text = self.text.slice(..);
        let line_index = text.line_idx_containing_byte(self.selection.cursor);
        let line = text.line_at(line_index);

        self.set_cursor(text.line_start_byte(line_index) + line.first_non_blank_offset());
    }

    fn move_cursor_next_paragraph(&mut self, count: usize) {
        for _ in 0..count {
            let text = self.text.slice(..);
            let line_index = text.line_idx_containing_byte(self.selection.cursor);

            let line_offset = self
                .text
                .lines_at(line_index.value(), LineType::LF_CR)
                .enumerate()
                .skip_while(|&(_i, line)| line.is_whitespace())
                .find(|&(_i, line)| line.is_whitespace())
                .map(|(i, _line)| i);

            self.set_cursor(match line_offset {
                Some(offset) => text.line_start_byte(line_index + offset),
                None => ByteIndex::new(text.len()),
            });
        }
    }

    fn move_cursor_prev_paragraph(&mut self, count: usize) {
        for _ in 0..count {
            let text = self.text.slice(..);
            let line_index = text.line_idx_containing_byte(self.selection.cursor);

            let line_offset = self
                .text
                // NOTE: +1 because when we use `reversed()`, the iterator does not consume the
                // line at the provided index
                .lines_at(line_index.value() + 1, LineType::LF_CR)
                .reversed()
                .enumerate()
                .skip_while(|&(_i, line)| line.is_whitespace())
                .find(|&(_i, line)| line.is_whitespace())
                .map(|(i, _line)| i);

            self.set_cursor(match line_offset {
                Some(offset) => text.line_start_byte(line_index.saturating_sub(offset)),
                None => ByteIndex::new(0),
            });
        }
    }

    fn go_to_last_line(&mut self) {
        let text = self.text.slice(..);

        self.set_cursor(text.line_start_byte(text.last_line_idx()));
    }

    const fn go_to_first_line(&mut self) {
        self.set_cursor(ByteIndex::new(0));
    }

    fn go_to_nth_or_first_line(&mut self, line_number: Option<NonZeroUsize>) {
        if let Some(line) = line_number {
            self.go_to_line_index(cmp::min(
                LineIndex::new(line.get() - 1),
                self.text.slice(..).last_line_idx(),
            ));
        } else {
            self.go_to_first_line();
        }
    }

    fn go_to_nth_or_last_line(&mut self, line_number: Option<NonZeroUsize>) {
        if let Some(line) = line_number {
            self.go_to_line_index(cmp::min(
                LineIndex::new(line.get() - 1),
                self.text.slice(..).last_line_idx(),
            ));
        } else {
            self.go_to_last_line();
        }
    }

    fn go_to_line_index(&mut self, line: LineIndex) {
        self.set_cursor(self.text.slice(..).line_start_byte(line));
    }

    fn content_layout(&self) -> ContentLayout {
        self.layout_info
            .content_layout(self.text.slice(..).line_count())
    }

    /// Deletes from the current cursor position up to (but not including) the
    /// start of the next word.
    fn delete_word(&self, count: usize) -> Transaction {
        let cursor = self.selection.cursor;

        let end = (0..count).fold(cursor, |start, _| {
            match self
                .text
                .slice(start.value()..)
                .chars()
                .tuple_windows()
                .map(|(left, right)| (LeftChar::new(left), RightChar::new(right)))
                .try_fold(start, |index, (left, right)| {
                    let next_index = index + left.ch().len_utf8();

                    if right.is_word_start(left) {
                        ControlFlow::Break(next_index)
                    } else {
                        ControlFlow::Continue(next_index)
                    }
                }) {
                ControlFlow::Continue(index) | ControlFlow::Break(index) => index,
            }
        });

        Transaction::new(Some(TextEdit::delete(cursor..end)), self.selection)
    }

    fn delete_to_line_end(&self) -> Transaction {
        let text = self.text.slice(..);

        let end = text
            .line_break(text.line_idx_containing_byte(self.selection.cursor))
            .position;

        Transaction::new(
            Some(TextEdit::delete(self.selection.cursor..end)),
            self.selection,
        )
    }

    fn delete_to_line_start(&self) -> Transaction {
        let text = self.text.slice(..);
        let line_start = text.line_start_byte(text.line_idx_containing_byte(self.selection.cursor));

        Transaction::new(
            Some(TextEdit::delete(text.inclusive_to_exclusive_range(
                line_start..=self.selection.cursor,
            ))),
            self.selection.with_cursor(line_start),
        )
    }

    fn delete_to_first_non_blank(&self) -> Transaction {
        let text = self.text.slice(..);
        let line_index = text.line_idx_containing_byte(self.selection.cursor);
        let line = text.line_at(line_index);

        let offset: ByteIndex = line
            .chars()
            .take_while(|ch| ch.is_whitespace())
            .map(|ch| ByteIndex::new(ch.len_utf8()))
            .sum();

        let start = text.line_start_byte(line_index) + offset;

        Transaction::new(
            Some(TextEdit::delete(
                text.inclusive_to_exclusive_range(start..=self.selection.cursor),
            )),
            self.selection.with_cursor(start),
        )
    }

    fn delete_line(&self, count: usize) -> Transaction {
        let range = self.line_range(count);
        let cursor = range.start;

        Transaction::new(
            Some(TextEdit::delete(range)),
            self.selection.with_cursor(cursor),
        )
    }

    fn delete_whole_word(&self) -> Transaction {
        let cursor = self.selection.cursor;
        let current_ch = self.text.char(cursor.value());

        let reversed_chars = self
            .text
            .slice(..cursor.value())
            .chars_at(cursor.value())
            .reversed();

        let start = iter::once(current_ch)
            .chain(reversed_chars)
            .tuple_windows()
            .map(|(right, left)| (LeftChar::new(left), RightChar::new(right)))
            .try_fold(cursor, |index, (left, right)| {
                if right.is_word_start(left) {
                    ControlFlow::Break(index)
                } else {
                    ControlFlow::Continue(index.saturating_sub(left.ch().len_utf8()))
                }
            })
            .break_value()
            .unwrap_or(ByteIndex::new(0));

        let end = match self
            .text
            .slice(cursor.value()..)
            .chars()
            .tuple_windows()
            .map(|(left, right)| (LeftChar::new(left), RightChar::new(right)))
            .try_fold(self.selection.cursor, |index, (left, right)| {
                let next_index = index + left.ch().len_utf8();

                if left.is_word_end(right) {
                    // we started at the leftmost byte of the `left` char, and we want to
                    // delete it, and so the byte index we provide is the start of the next
                    // char, allowing us to use an exclusive range in the `remove` call below
                    ControlFlow::Break(next_index)
                } else {
                    ControlFlow::Continue(next_index)
                }
            }) {
            ControlFlow::Continue(index) | ControlFlow::Break(index) => index,
        };

        Transaction::new(
            Some(TextEdit::delete(start..end)),
            self.selection.with_cursor(start),
        )
    }

    fn delete_to_prev_word_start(&self, count: usize) -> Transaction {
        let cursor = self.selection.cursor;
        let text = self.text.slice(..cursor.value());

        let start = match count {
            0 => cursor,
            _ => {
                text.chars_at(cursor.value())
                    .reversed()
                    .tuple_windows()
                    .map(|(right, left)| (LeftChar::new(left), RightChar::new(right)))
                    .scan(cursor, |index, (left, right)| {
                        *index = index.saturating_sub(left.ch().len_utf8());
                        Some(right.is_word_start(left).then_some(*index))
                    })
                    .flatten()
                    .nth(count - 1)
                    .unwrap_or(ByteIndex::new(0))
            }
        };

        Transaction::new(
            Some(TextEdit::delete(start..cursor)),
            self.selection.with_cursor(start),
        )
    }

    fn append_text(&mut self) {
        self.move_cursor_right(1);
        self.insert_mode();
    }

    fn append_text_line_end(&self) -> Transaction {
        let text = self.text.slice(..);
        let line_break = text.line_break(text.line_idx_containing_byte(self.selection.cursor));

        let edit = if line_break.has_linebreak {
            None
        } else {
            // there is no linebreak, and so we need to make room to append text by adding
            // one. we will not shift the cursor, so the user will overwrite the
            // empty space when they enter text
            // TODO: use the same linebreak style that the rest of the document uses, if
            // applicable
            Some(TextEdit::insert(line_break.position, '\n'))
        };

        Transaction::new(edit, self.selection.with_cursor(line_break.position))
    }

    fn move_cursor_word_end(&mut self, count: usize) {
        for _ in 0..count {
            // we start searching at the next grapheme so that the cursor doesn't stay where
            // it is if it's already at the end of a word (in that case, we want to
            // go to the end of the **next** word)
            let search_start = self
                .text
                .slice(..)
                .next_grapheme_position(self.selection.cursor);

            let word_end = match self
                .text
                .slice(search_start.value()..)
                .chars()
                .tuple_windows()
                .map(|(left, right)| (LeftChar::new(left), RightChar::new(right)))
                .try_fold(search_start, |index, (left, right)| {
                    if left.is_word_end(right) {
                        ControlFlow::Break(index)
                    } else {
                        ControlFlow::Continue(index + left.ch().len_utf8())
                    }
                }) {
                ControlFlow::Continue(index) | ControlFlow::Break(index) => index,
            };

            self.set_cursor(word_end);
        }
    }

    fn delete_to_word_end(&self, count: usize) -> Transaction {
        let cursor = self.selection.cursor;

        let end = (0..count).fold(cursor, |start, _| {
            // we start searching at the next grapheme so that the cursor doesn't stay where
            // it is if it's already at the end of a word (in that case, we want to
            // go to the end of the **next** word)
            let search_start = self.text.slice(..).next_grapheme_position(start);

            match self
                .text
                .slice(search_start.value()..)
                .chars()
                .tuple_windows()
                .map(|(left, right)| (LeftChar::new(left), RightChar::new(right)))
                .try_fold(search_start, |index, (left, right)| {
                    let next_index = index + left.ch().len_utf8();

                    if left.is_word_end(right) {
                        // we want to delete the whole character, which may be multiple bytes,
                        // and so we delete up to (but not including) the next character index
                        ControlFlow::Break(next_index)
                    } else {
                        ControlFlow::Continue(next_index)
                    }
                }) {
                ControlFlow::Continue(index) | ControlFlow::Break(index) => index,
            }
        });

        Transaction::new(Some(TextEdit::delete(cursor..end)), self.selection)
    }

    fn change_line(&self, count: usize) -> Transaction {
        let range = self.line_range(count);
        let cursor = range.start;

        Transaction::new(
            // we removed the linebreak (if there was one), and so we need to make room to
            // append text by adding one. we will not shift the cursor, so the user
            // will overwrite the empty space when they enter text
            // TODO: use the same linebreak style that the rest of the document uses, if
            // applicable
            Some(TextEdit::replace(range, "\n")),
            self.selection.with_cursor(cursor),
        )
    }

    fn delete_selection(&self) -> Transaction {
        let range = self.selection.range(self.text.slice(..));
        let cursor = range.start;

        Transaction::new(
            Some(TextEdit::delete(range)),
            self.selection.with_cursor(cursor),
        )
    }

    const fn reverse_selection(&mut self) {
        self.selection.reverse();
    }

    fn open_new_line_below(&self) -> Transaction {
        let text = self.text.slice(..);

        let line_index = text.line_idx_containing_byte(self.selection.cursor);
        let line_break = text.line_break(line_index);

        let to_insert = if line_break.has_linebreak {
            "\n"
        } else {
            "\n\n"
        };

        Transaction::new(
            Some(TextEdit::insert(line_break.position, to_insert)),
            self.selection
                .with_cursor(line_break.position + '\n'.len_utf8()),
        )
    }

    fn open_new_line_above(&self) -> Transaction {
        let text = self.text.slice(..);

        let line_start = text.line_start_byte(text.line_idx_containing_byte(self.selection.cursor));

        Transaction::new(
            Some(TextEdit::insert(line_start, '\n')),
            self.selection.with_cursor(line_start),
        )
    }

    fn select_current_word(&mut self) {
        let current_ch = self.text.char(self.selection.cursor.value());

        let reversed_chars = self
            .text
            .slice(..self.selection.cursor.value())
            .chars_at(self.selection.cursor.value())
            .reversed();

        let start = iter::once(current_ch)
            .chain(reversed_chars)
            .tuple_windows()
            .map(|(right, left)| (LeftChar::new(left), RightChar::new(right)))
            .try_fold(self.selection.cursor, |index, (left, right)| {
                if right.is_word_start(left) {
                    ControlFlow::Break(index)
                } else {
                    ControlFlow::Continue(index.saturating_sub(left.ch().len_utf8()))
                }
            })
            .break_value()
            .unwrap_or(ByteIndex::new(0));

        let end = match self
            .text
            .slice(self.selection.cursor.value()..)
            .chars()
            .tuple_windows()
            .map(|(left, right)| (LeftChar::new(left), RightChar::new(right)))
            .try_fold(self.selection.cursor, |index, (left, right)| {
                let next_index = index + left.ch().len_utf8();

                if left.is_word_end(right) {
                    ControlFlow::Break(index)
                } else {
                    ControlFlow::Continue(next_index)
                }
            }) {
            ControlFlow::Continue(index) | ControlFlow::Break(index) => index,
        };

        self.set_anchor(start);
        self.set_cursor(end);
    }

    /// Deletes the current plus the `count` succeeding lines.
    fn delete_down(&self, count: usize) -> Transaction {
        let text = self.text.slice(..);

        let range = self.line_range(count + 1);
        let next_line = text.slice(range.end.value()..).line_at(LineIndex::new(0));
        let cursor = range.start + next_line.first_non_blank_offset();

        Transaction::new(
            Some(TextEdit::delete(range)),
            self.selection.with_cursor(cursor),
        )
    }

    /// Deletes the current plus the `count` preceding lines.
    fn delete_up(&self, count: usize) -> Transaction {
        let text = self.text.slice(..);
        let current_line = text.line_idx_containing_byte(self.selection.cursor);

        if current_line == LineIndex::new(0) {
            return Transaction::new(None, self.selection);
        }

        let range = self.backwards_line_range(count + 1);
        let cursor = text
            .line_idx_containing_byte(range.start)
            .checked_sub(1)
            .map_or(range.start, |line| {
                text.line_start_byte(line) + text.line_at(line).first_non_blank_offset()
            });

        Transaction::new(
            Some(TextEdit::delete(range)),
            self.selection.with_cursor(cursor),
        )
    }

    fn open_command_list(event_context: &mut EventContext) {
        event_context.push_action(EditorAction::AddLayer(LayerKind::CommandList));
    }

    fn open_file_picker(event_context: &mut EventContext) {
        event_context.push_action(EditorAction::AddLayer(LayerKind::FilePicker));
    }

    fn visual_cursor_position_impl(&self) -> Position {
        let content_layout = self.content_layout();

        let Some(text_width) = content_layout.max_text_width() else {
            return Position::default();
        };

        let start = self.text.slice(..).line_start_byte(self.scroll_offset);

        GraphemeLayoutIterator::new(
            self.text.slice(start.value()..).graphemes(),
            WrapBehavior::Wrap {
                max_width: text_width,
            },
        )
        .find(|grapheme| start + grapheme.byte_index() >= self.selection.cursor)
        .map(|grapheme| grapheme.position())
        .unwrap_or_default()
        .col_offset(content_layout.gutter.width)
    }

    pub(crate) fn save(&self) -> anyhow::Result<()> {
        // TODO: atomic saves
        // TODO: async saves

        self.text
            .write_to(BufWriter::new(
                File::create(&self.file_path).context("File cannot be written to")?,
            ))
            .context("Failed to save")?;

        Ok(())
    }

    pub(crate) fn set_error(&mut self, error_message: String) {
        self.error = Some(error_message);
    }

    fn clear_input(&mut self) {
        self.key_sequence.clear();
    }

    const fn mode(&self) -> Mode {
        self.key_sequence.mode()
    }

    fn insert_tab(&self) -> Transaction {
        // TODO: if the file treats tabs as spaces, then we should insert spaces here
        // instead.
        self.insert_char('\t')
    }

    /// Gets the range of bytes for `count` lines, starting with the current
    /// line.
    fn line_range(&self, count: usize) -> Range<ByteIndex> {
        let text = self.text.slice(..);
        let index = text.line_idx_containing_byte(self.selection.cursor);
        let start = text.line_start_byte(index);
        let end = text
            .get_line_start_byte(index + count)
            .unwrap_or_else(|| ByteIndex::new(text.len()));

        start..end
    }

    /// Gets the range of bytes for `count` lines, going backwards from the
    /// current line.
    fn backwards_line_range(&self, count: usize) -> Range<ByteIndex> {
        let text = self.text.slice(..);
        let index_after_current = text.line_idx_containing_byte(self.selection.cursor) + 1;
        let start = text.line_start_byte(index_after_current.saturating_sub(count));
        let end = text
            .get_line_start_byte(index_after_current)
            .unwrap_or_else(|| ByteIndex::new(text.len()));

        start..end
    }

    fn request_highlight_refresh(&self) {
        let _ = self.highlight_request_tx.send(HighlightRequest {
            text: self.text.clone(),
            language: self.language,
        });
    }

    pub(crate) fn set_jj_info(&mut self, info: JJInfo) {
        self.jj_info = info;
    }

    fn go_to_pair_match(&mut self) {
        let cursor = self.selection.cursor.value();

        let Some(pair) = self.text.get_byte(cursor).and_then(PairItem::new) else {
            return;
        };

        let current = pair.as_byte();
        let opposite = pair.opposite_as_byte();

        assert_ne!(current, opposite, "current should never be opposite!");

        let bytes = match pair.position {
            PairPosition::Start => self.text.bytes_at(cursor + 1),
            PairPosition::End => self.text.bytes_at(cursor).reversed(),
        };

        let Some(offset) = bytes
            .enumerate()
            .try_fold(1_u64, |depth, (offset, byte)| {
                let next_depth = if byte == current {
                    depth + 1
                } else if byte == opposite {
                    depth - 1
                } else {
                    depth
                };

                if next_depth == 0 {
                    ControlFlow::Break(ByteIndex::new(offset + 1))
                } else {
                    ControlFlow::Continue(next_depth)
                }
            })
            .break_value()
        else {
            return;
        };

        self.set_cursor(match pair.position {
            PairPosition::Start => self.selection.cursor + offset,
            PairPosition::End => self.selection.cursor - offset,
        });
    }

    /// Creates a snapshot of relevant data from the document to be used in
    /// requests made to language server(s).
    pub(crate) fn lsp_snapshot(&self) -> DocumentSnapshot {
        DocumentSnapshot::new(self.lsp_id.clone(), self.text.clone(), self.version)
    }

    pub(crate) const fn url(&self) -> &Url {
        self.lsp_id.url()
    }

    pub(crate) fn publish_diagnostics(
        &mut self,
        params: &PublishDiagnosticsParams,
        position_encoding: PositionEncoding,
    ) {
        let text = self.text.slice(..);

        self.diagnostics = params
            .diagnostics
            .iter()
            .filter_map(|diagnostic| {
                Diagnostic::new(diagnostic, position_encoding, text)
                    .inspect_err(|err| log::warn!("invalid diagnostic: {err}"))
                    .ok()
            })
            .collect();
    }

    pub(crate) const fn version(&self) -> DocumentVersion {
        self.version
    }

    pub(crate) const fn lsp_id(&self) -> &DocumentLspId {
        &self.lsp_id
    }
}

fn spawn_highlights_thread() -> (Sender<HighlightRequest>, Receiver<HighlightCache>) {
    let (highlight_request_tx, highlight_request_rx) = sync::mpsc::channel::<HighlightRequest>();
    let (highlight_response_tx, highlight_response_rx) = sync::mpsc::channel::<HighlightCache>();

    thread::spawn(move || {
        while let Ok(request) = highlight_request_rx.recv() {
            let request = highlight_request_rx.try_iter().last().unwrap_or(request);

            let _ = highlight_response_tx.send(HighlightCache {
                tokens: Highlighter::new(request.text, ByteIndex::new(0), request.language)
                    .filter_map(|(token, checkpoint)| {
                        match checkpoint {
                            Checkpoint::Yes => Some(token),
                            Checkpoint::No => None,
                        }
                    })
                    .collect(),
            });
        }
    });

    (highlight_request_tx, highlight_response_rx)
}

impl Layer for Document {
    fn render(&mut self, buffer: &mut Buffer) {
        let text = self.text.slice(..);

        let start_byte = text.line_start_byte(self.scroll_offset);

        buffer.clear_and_style_rectangle(&self.layout_info.content_rect, Style::BACKGROUND);

        let content_layout = self.content_layout();
        let gutter_width = content_layout.gutter.width;

        let Some(text_width) = content_layout.max_text_width() else {
            // if there's no room for text, there's no point rendering anything.
            return;
        };

        let cursor_line = text.line_idx_containing_byte(self.selection.cursor);

        let mut line_index = self.scroll_offset;

        let mut highlighter = Highlighter::new(
            self.text.clone(),
            self.highlights.checkpoint_before(start_byte),
            self.language,
        );
        let mut current_highlight = highlighter.next();

        let gutter_renderer = GutterRenderer {
            gutter: &content_layout.gutter,
            cursor_line,
        };

        for visual_grapheme in GraphemeLayoutIterator::new(
            self.text.slice(start_byte.value()..).graphemes(),
            WrapBehavior::Wrap {
                max_width: text_width,
            },
        ) {
            if visual_grapheme.position().top() >= self.layout_info.content_rect.height() {
                break;
            }

            if visual_grapheme.position().left() == Columns::new(0) {
                gutter_renderer.render(
                    visual_grapheme.position(),
                    visual_grapheme.wrap_status(),
                    line_index,
                    buffer,
                );
            }

            let translated_position = visual_grapheme.position().col_offset(gutter_width);

            assert!(
                translated_position.left() >= gutter_width,
                "filling in the gutter should've taken the position past the gutter"
            );

            let grapheme = visual_grapheme.grapheme();

            let grapheme_index = start_byte + visual_grapheme.byte_index();

            while current_highlight
                .as_ref()
                .is_some_and(|&(ref highlight, _checkpoint)| !highlight.contains(grapheme_index))
            {
                current_highlight = highlighter.next();
            }

            let style = current_highlight
                .as_ref()
                .map(|&(ref token, _checkpoint)| token.kind())
                .map_or(Style::TEXT, Style::from)
                .merge(
                    self.diagnostics
                        .iter()
                        .find(|diagnostic| {
                            // some syntax errors come back as an empty range, so we handle those
                            // explicitly.
                            if diagnostic.range.is_empty() {
                                diagnostic.range.start == grapheme_index
                            } else {
                                diagnostic.range.contains(&grapheme_index)
                            }
                        })
                        .map_or_else(Style::new, |diagnostic| {
                            Style::diagnostic(diagnostic.severity)
                        }),
                );

            buffer[translated_position]
                .set_content(grapheme.as_str())
                .set_style(style);

            if matches!(self.mode(), Mode::Visual)
                && self.selection.range(text).contains(&grapheme_index)
            {
                buffer[translated_position].set_style(style.merge(Style::TEXT_SELECTED));
            }

            if matches!(grapheme, Grapheme::LineBreak) {
                line_index += LineIndex::new(1);
            }
        }

        self.render_status_line(buffer);
        self.render_key_hint(buffer);
    }

    fn handle_event(&mut self, event: &Event, event_context: &mut EventContext) -> EventOutcome {
        match *event {
            Event::Key(key_event) => self.handle_key_event(key_event, event_context),
            Event::Mouse(_mouse_event) => todo!(),
            Event::Resize(columns, rows) => {
                self.resize(Dimensions::new(Columns::from(columns), Rows::from(rows)));
                EventOutcome::Handled
            }

            Event::FocusGained | Event::FocusLost | Event::Paste(_) => todo!(),
        }
    }

    fn visual_cursor_position(&self) -> Option<Position> {
        Some(self.visual_cursor_position_impl())
    }

    fn handle_internal_events(&mut self) -> EventOutcome {
        match self.highlight_response_rx.try_iter().last() {
            Some(highlights) => {
                self.highlights = highlights;
                EventOutcome::Handled
            }
            None => EventOutcome::Unhandled,
        }
    }

    fn kind(&self) -> Option<LayerKind> {
        None
    }
}

#[derive(Debug)]
pub(crate) struct LayoutInfo {
    /// Size and position of the area that the file contents (including
    /// gutters) can be rendered into.
    content_rect: Rectangle,
    /// Size and position of the area that the status line can be rendered
    /// into.
    status_line_rect: Rectangle,
}

impl LayoutInfo {
    fn new(dimensions: Dimensions) -> Self {
        let (content_rect, status_line_rect) = Rectangle::from_dimensions(dimensions)
            .split_at_row(dimensions.height().saturating_sub(Rows::new(2)));

        Self {
            content_rect,
            status_line_rect,
        }
    }

    fn content_layout(&self, line_count: usize) -> ContentLayout {
        ContentLayout::new(&self.content_rect, line_count)
    }
}

#[derive(Debug)]
struct ContentLayout {
    gutter: GutterRow,
    text_width: Columns,
}

impl ContentLayout {
    fn new(container: &Rectangle, line_count: usize) -> Self {
        let gutter_width = cmp::max(
            Columns::new(3),
            GutterRow::LINE_NUMBER_RIGHT_PADDING + number_of_digits(line_count),
        );

        Self {
            gutter: GutterRow {
                width: gutter_width,
            },
            text_width: container.width().saturating_sub(gutter_width),
        }
    }

    fn max_text_width(&self) -> Option<NonZeroColumns> {
        NonZeroColumns::new(self.text_width)
    }
}

#[derive(Debug)]
struct GutterRow {
    width: Columns,
}

impl GutterRow {
    const LINE_NUMBER_RIGHT_PADDING: Columns = Columns::new(1);
}

#[derive(Debug)]
struct GutterRenderer<'gutter> {
    gutter: &'gutter GutterRow,
    cursor_line: LineIndex,
}

impl GutterRenderer<'_> {
    /// Renders the gutter for the current visual line (derived from line index
    /// and wrap status). Must be called at the start of a visual line.
    fn render(
        &self,
        position: Position,
        wrapped: WrapOutcome,
        current_line: LineIndex,
        buffer: &mut Buffer,
    ) {
        assert_eq!(
            position.left(),
            Columns::new(0),
            "gutter must start at the left edge of the screen"
        );

        let gutter_contents = match wrapped {
            // we only display the line number on the first visual row of a wrapped
            // line
            WrapOutcome::Wrapped => " ".repeat(self.gutter.width.value()),
            WrapOutcome::NotWrapped => {
                let line_number_width = self
                    .gutter
                    .width
                    .saturating_sub(GutterRow::LINE_NUMBER_RIGHT_PADDING);

                let line_number = if self.cursor_line == current_line {
                    current_line + 1
                } else {
                    current_line.abs_diff(self.cursor_line)
                };

                format!(
                    "{line_number:>width$}{}",
                    " ".repeat(GutterRow::LINE_NUMBER_RIGHT_PADDING.value()),
                    width = line_number_width.value(),
                )
            }
        };

        let gutter_style = if self.cursor_line == current_line {
            Style::GUTTER_SELECTED
        } else {
            Style::GUTTER
        };

        buffer[position]
            .set_content(&gutter_contents)
            .set_style(gutter_style);
    }
}

#[derive(Debug)]
pub(crate) enum Grapheme<'grapheme> {
    LineBreak,
    Tab,
    Text(&'grapheme str),
}

const fn tab_str() -> &'static str {
    match str::from_utf8(&[b' '; TAB_VISUAL_WIDTH.value()]) {
        Ok(s) => s,
        Err(_) => unreachable!(),
    }
}

impl Grapheme<'_> {
    const TAB_STR: &'static str = tab_str();

    pub(crate) const fn as_str(&self) -> &str {
        match *self {
            Grapheme::LineBreak => " ",
            Grapheme::Tab => Self::TAB_STR,
            Grapheme::Text(text) => text,
        }
    }
}

impl<'grapheme> From<&'grapheme str> for Grapheme<'grapheme> {
    fn from(value: &'grapheme str) -> Self {
        match value {
            "\n" | "\r\n" => Self::LineBreak,
            "\t" => Self::Tab,
            _ => Self::Text(value),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Selection {
    /// The "start" of the selection. This is set to the current cursor position
    /// when entering [`Mode::Visual`] and then does not change while in that
    /// mode. If the document is not [`Mode::Visual`] then the value of this
    /// field is meaningless.
    anchor: ByteIndex,
    /// The "end" of the selection, which changes during movement. It is **not**
    /// restricted to appearing after [`Selection::anchor`]: it can overlap it,
    /// or appear before it in the document.
    cursor: ByteIndex,
}

impl Selection {
    /// Gets the range of bytes that the selection represents.
    fn range(&self, text: RopeSlice) -> Range<ByteIndex> {
        let start = cmp::min(self.cursor, self.anchor);
        let end = cmp::max(self.cursor, self.anchor);

        // since each byte index represents the **start** of a grapheme, in order to get
        // all of the selected bytes, we extend the rightmost index to the start
        // of the **next** grapheme and represent it as a half-open range.
        text.inclusive_to_exclusive_range(start..=end)
    }

    /// Creates a new [`Selection`] with the cursor set to the given position.
    #[must_use]
    const fn with_cursor(self, cursor: ByteIndex) -> Self {
        Self { cursor, ..self }
    }

    const fn reverse(&mut self) {
        mem::swap(&mut self.anchor, &mut self.cursor);
    }
}

fn number_of_digits(value: usize) -> usize {
    (value.checked_ilog10().unwrap_or(0) + 1) as usize
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Mode {
    Normal,
    Insert,
    Visual,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", match *self {
            Self::Normal => "NORMAL",
            Self::Insert => "INSERT",
            Self::Visual => "VISUAL",
        })
    }
}

#[derive(Debug)]
struct HighlightCache {
    tokens: Vec<Token>,
}

impl HighlightCache {
    const fn new() -> Self {
        Self { tokens: Vec::new() }
    }

    /// Gets the start byte of the closest token that occurs before the given
    /// [`ByteIndex`]. This can be used as a start point to re-calculate the
    /// highlights for a section of the text.
    fn checkpoint_before(&self, index: ByteIndex) -> ByteIndex {
        // if the token cache isn't yet populated, we don't want to highlight the whole
        // text as this could be slow for extremely large files and this method
        // blocks the renderer. instead, we'll just act as if `index` is a
        // checkpoint and allow the highlights to potentially be slightly off until the
        // token cache populates (shouldn't actually be noticeable to the user)
        if self.tokens.is_empty() {
            return index;
        }

        self.tokens
            .get(
                self.tokens
                    .partition_point(|token| token.end() < index)
                    .saturating_sub(1),
            )
            .map_or(ByteIndex::new(0), Token::start)
    }
}

#[derive(Debug)]
struct HighlightRequest {
    text: Rope,
    language: Language,
}

#[derive(Debug)]
struct PairItem {
    kind: PairKind,
    position: PairPosition,
}

impl PairItem {
    const fn new(value: u8) -> Option<Self> {
        match value {
            b'(' => {
                Some(Self {
                    kind: PairKind::Paren,
                    position: PairPosition::Start,
                })
            }
            b')' => {
                Some(Self {
                    kind: PairKind::Paren,
                    position: PairPosition::End,
                })
            }
            b'{' => {
                Some(Self {
                    kind: PairKind::Brace,
                    position: PairPosition::Start,
                })
            }
            b'}' => {
                Some(Self {
                    kind: PairKind::Brace,
                    position: PairPosition::End,
                })
            }
            b'[' => {
                Some(Self {
                    kind: PairKind::Bracket,
                    position: PairPosition::Start,
                })
            }
            b']' => {
                Some(Self {
                    kind: PairKind::Bracket,
                    position: PairPosition::End,
                })
            }
            _ => None,
        }
    }

    const fn as_byte(&self) -> u8 {
        match self.position {
            PairPosition::Start => self.kind.start_byte(),
            PairPosition::End => self.kind.end_byte(),
        }
    }

    const fn opposite_as_byte(&self) -> u8 {
        match self.position {
            PairPosition::Start => self.kind.end_byte(),
            PairPosition::End => self.kind.start_byte(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum PairKind {
    /// `(` or `)`.
    Paren,
    /// `{` or `}`.
    Brace,
    /// `[` or `]`.
    Bracket,
}

impl PairKind {
    const fn start_byte(self) -> u8 {
        match self {
            Self::Paren => b'(',
            Self::Brace => b'{',
            Self::Bracket => b'[',
        }
    }

    const fn end_byte(self) -> u8 {
        match self {
            Self::Paren => b')',
            Self::Brace => b'}',
            Self::Bracket => b']',
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum PairPosition {
    Start,
    End,
}

#[derive(Debug)]
struct Transaction {
    edit: Option<TextEdit>,
    selection: Selection,
}

impl Transaction {
    const fn new(edit: Option<TextEdit>, selection: Selection) -> Self {
        Self { edit, selection }
    }
}

#[derive(Debug)]
pub(crate) struct TextEdit {
    range: Range<ByteIndex>,
    replacement: String,
}

impl TextEdit {
    fn insert(at: ByteIndex, text: impl Into<String>) -> Self {
        Self::replace(at..at, text)
    }

    fn delete(range: Range<ByteIndex>) -> Self {
        Self::replace(range, "")
    }

    fn replace(range: Range<ByteIndex>, text: impl Into<String>) -> Self {
        assert!(range.start <= range.end, "range must be valid");

        Self {
            range,
            replacement: text.into(),
        }
    }

    pub(crate) fn into_parts(self) -> (Range<ByteIndex>, String) {
        (self.range, self.replacement)
    }

    fn is_noop(&self) -> bool {
        self.range.is_empty() && self.replacement.is_empty()
    }
}

#[derive(Debug)]
struct Diagnostic {
    severity: DiagnosticSeverity,
    range: Range<ByteIndex>,
}

impl Diagnostic {
    fn new(
        diagnostic: &gen_lsp_types::Diagnostic,
        position_encoding: PositionEncoding,
        text: RopeSlice<'_>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            severity: diagnostic.severity.unwrap_or(DiagnosticSeverity::Error),
            range: lsp_to_byte_range(diagnostic.range, position_encoding, text)?,
        })
    }
}

fn lsp_to_byte_range(
    range: gen_lsp_types::Range,
    position_encoding: PositionEncoding,
    text: RopeSlice<'_>,
) -> anyhow::Result<Range<ByteIndex>> {
    let translate_position = |position: gen_lsp_types::Position| -> anyhow::Result<ByteIndex> {
        let line_index = LineIndex::try_from(position.line).context("line index is too large")?;

        let line = text
            .get_line_at(line_index)
            .context("invalid line index for this document")?;
        let character =
            usize::try_from(position.character).context("character offset is too large")?;

        let byte_offset = match position_encoding {
            PositionEncoding::UTF8 => character,
            // TODO: use non-panicking versions when <https://github.com/cessen/ropey/pull/118> is
            // done.
            PositionEncoding::UTF16 => line.utf16_to_byte_idx(character),
            PositionEncoding::UTF32 => line.char_to_byte_idx(character),
        };

        Ok(text.line_start_byte(line_index) + byte_offset)
    };
    let start = translate_position(range.start)?;
    let end = translate_position(range.end)?;

    anyhow::ensure!(start <= end, "diagnostic range is invalid: end > start");

    Ok(start..end)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use crossterm::event::{
        KeyCode,
        KeyEvent,
    };

    use super::*;

    macro_rules! key_event {
        ($key:literal) => {
            KeyEvent::from(KeyCode::Char($key))
        };

        ($key:ident) => {
            KeyEvent::from(KeyCode::$key)
        };
    }

    const TEST_DIMENSIONS: Dimensions = Dimensions::new(Columns::new(80), Rows::new(24));

    struct TestCase<'text> {
        initial_text: &'text str,
        initial_cursor: usize,
        expected_initial_text_position: (usize, usize),

        keys: Vec<KeyEvent>,

        expected_text: &'text str,
        expected_cursor: usize,
        expected_text_position: (usize, usize),
    }

    impl TestCase<'_> {
        fn run(self) {
            let _ = color_eyre::install();

            let mut document = doc(self.initial_text);

            document.set_cursor(self.initial_cursor.into());
            assert_text_position(
                self.expected_initial_text_position,
                "initial position",
                &document,
            );
            assert_char_boundary(&document);

            for event in self.keys {
                let _ = document.handle_key_event(event, &mut EventContext::new());
                assert_char_boundary(&document);
            }

            assert_eq!(
                document.text.to_string(),
                self.expected_text,
                "text is incorrect"
            );
            assert_eq!(
                document.selection.cursor,
                self.expected_cursor.into(),
                "cursor byte index is incorrect"
            );
            assert_text_position(self.expected_text_position, "final position", &document);
            assert_char_boundary(&document);
        }
    }

    fn assert_text_position(expected: (usize, usize), label: &str, document: &Document) {
        let actual = document.visual_cursor_position().unwrap();

        let expected = Position::new(Columns::new(expected.0), Rows::new(expected.1))
            .col_offset(document.content_layout().gutter.width);

        assert_eq!(actual, expected, "{label} did not match");
    }

    fn assert_char_boundary(doc: &Document) {
        assert!(
            doc.text.is_char_boundary(doc.selection.cursor.value()),
            "cursor isn't a char boundary"
        );
    }

    fn doc(contents: &str) -> Document {
        let mut temp_file = tempfile::NamedTempFile::new().unwrap();
        write!(temp_file, "{contents}").unwrap();

        Document::new(temp_file.path().to_path_buf(), TEST_DIMENSIONS).unwrap()
    }

    #[test]
    fn move_cursor_right() {
        TestCase {
            initial_text: "Test ⚒️ 😀 ",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('l'); 8],

            expected_text: "Test ⚒️ 😀 ",
            expected_cursor: 16,
            expected_text_position: (10, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_right_from_end() {
        TestCase {
            initial_text: "Test",
            initial_cursor: 3,
            expected_initial_text_position: (3, 0),

            keys: vec![key_event!('l'); 1],

            expected_text: "Test",
            expected_cursor: 3,
            expected_text_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_right_n() {
        TestCase {
            initial_text: "Test ⚒️ 😀 ",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('8'), key_event!('l')],

            expected_text: "Test ⚒️ 😀 ",
            expected_cursor: 16,
            expected_text_position: (10, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_left() {
        TestCase {
            initial_text: "Test ⚒️ 😀 ",
            initial_cursor: 16,
            expected_initial_text_position: (10, 0),

            keys: vec![key_event!('h'); 1],

            expected_text: "Test ⚒️ 😀 ",
            expected_cursor: 12,
            expected_text_position: (8, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_left_from_start() {
        TestCase {
            initial_text: "Test",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('h'); 1],

            expected_text: "Test",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_left_n() {
        TestCase {
            initial_text: "Test ⚒️ 😀 ",
            initial_cursor: 16,
            expected_initial_text_position: (10, 0),

            keys: vec![key_event!('4'), key_event!('h')],

            expected_text: "Test ⚒️ 😀 ",
            expected_cursor: 4,
            expected_text_position: (4, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_down() {
        TestCase {
            initial_text: "Test\nTest",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('j'); 1],

            expected_text: "Test\nTest",
            expected_cursor: 5,
            expected_text_position: (0, 1),
        }
        .run();
    }

    #[test]
    fn move_cursor_down_from_bottom() {
        TestCase {
            initial_text: "Test\nTest",
            initial_cursor: 5,
            expected_initial_text_position: (0, 1),

            keys: vec![key_event!('j'); 1],

            expected_text: "Test\nTest",
            expected_cursor: 5,
            expected_text_position: (0, 1),
        }
        .run();
    }

    #[test]
    fn move_cursor_down_from_bottom_trailing_newline() {
        TestCase {
            initial_text: "Test\nTest\n",
            initial_cursor: 5,
            expected_initial_text_position: (0, 1),

            keys: vec![key_event!('j'); 1],

            expected_text: "Test\nTest\n",
            expected_cursor: 5,
            expected_text_position: (0, 1),
        }
        .run();
    }

    #[test]
    fn move_cursor_down_wrapped() {
        TestCase {
            initial_text: &"a".repeat(200),
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('j')],

            expected_text: &"a".repeat(200),
            expected_cursor: 77,
            expected_text_position: (0, 1),
        }
        .run();
    }

    #[test]
    fn move_cursor_down_n() {
        TestCase {
            initial_text: "Test\nTest\nTest",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![key_event!('2'), key_event!('j')],

            expected_text: "Test\nTest\nTest",
            expected_cursor: 12,
            expected_text_position: (2, 2),
        }
        .run();
    }

    #[test]
    fn move_cursor_down_n_multiple_digits() {
        TestCase {
            initial_text: &"Test\n".repeat(15),
            initial_cursor: 3,
            expected_initial_text_position: (3, 0),

            keys: vec![key_event!('1'), key_event!('2'), key_event!('j')],

            expected_text: &"Test\n".repeat(15),
            expected_cursor: 63,
            expected_text_position: (3, 12),
        }
        .run();
    }

    #[test]
    fn move_cursor_up() {
        TestCase {
            initial_text: "Test\nTest",
            initial_cursor: 5,
            expected_initial_text_position: (0, 1),

            keys: vec![key_event!('k'); 1],

            expected_text: "Test\nTest",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_up_from_top() {
        TestCase {
            initial_text: "Test\nTest",
            initial_cursor: 1,
            expected_initial_text_position: (1, 0),

            keys: vec![key_event!('k'); 1],

            expected_text: "Test\nTest",
            expected_cursor: 1,
            expected_text_position: (1, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_up_wrapped() {
        TestCase {
            initial_text: &"a".repeat(200),
            initial_cursor: 77,
            expected_initial_text_position: (0, 1),

            keys: vec![key_event!('k')],

            expected_text: &"a".repeat(200),
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_up_n() {
        TestCase {
            initial_text: "Test\nTest\nTest",
            initial_cursor: 11,
            expected_initial_text_position: (1, 2),

            keys: vec![key_event!('2'), key_event!('k')],

            expected_text: "Test\nTest\nTest",
            expected_cursor: 1,
            expected_text_position: (1, 0),
        }
        .run();
    }

    #[test]
    fn maintain_column_up_multiple_lines() {
        TestCase {
            initial_text: "Long line\nShort\nLong line",
            initial_cursor: 24,
            expected_initial_text_position: (8, 2),

            keys: vec![key_event!('k'); 1],

            expected_text: "Long line\nShort\nLong line",
            expected_cursor: 15,
            expected_text_position: (5, 1),
        }
        .run();
    }

    #[test]
    fn scroll_down() {
        let text = "Test\n".repeat(30);

        TestCase {
            initial_text: &text,
            initial_cursor: 21 * 5,
            expected_initial_text_position: (0, 21),

            keys: vec![key_event!('j'); 1],

            expected_text: &text,
            expected_cursor: 22 * 5,
            // visual position stays the same because we scrolled down, keeping the
            // cursor on the final line
            expected_text_position: (0, 21),
        }
        .run();
    }

    #[test]
    fn scroll_down_wrapped() {
        let line = "a".repeat(200);
        let text = format!("{line}\n").repeat(100);

        TestCase {
            initial_text: &text,
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('G'); 1],

            expected_text: &text,
            expected_cursor: (200 * 99) + 99,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn scroll_up() {
        let text = "Test\n".repeat(30);

        TestCase {
            initial_text: &text,
            initial_cursor: 1,
            expected_initial_text_position: (1, 0),

            keys: [vec![key_event!('j'); 26], vec![key_event!('k'); 25]].concat(),

            expected_text: &text,
            expected_cursor: 6,
            // visual position stays the same because we scrolled up, keeping the
            // cursor on the first line
            expected_text_position: (1, 0),
        }
        .run();
    }

    #[test]
    fn center_cursor_vertically_normal_mode() {
        let text = "Test\n".repeat(40);

        TestCase {
            initial_text: &text,
            initial_cursor: 20 * 5,
            expected_initial_text_position: (0, 20),

            keys: vec![key_event!('z'), key_event!('z')],

            expected_text: &text,
            expected_cursor: 20 * 5,
            expected_text_position: (0, 11),
        }
        .run();
    }

    #[test]
    fn center_cursor_vertically_visual_mode() {
        let text = "Test\n".repeat(40);

        TestCase {
            initial_text: &text,
            initial_cursor: 20 * 5,
            expected_initial_text_position: (0, 20),

            keys: vec![key_event!('v'), key_event!('z'), key_event!('z')],

            expected_text: &text,
            expected_cursor: 20 * 5,
            expected_text_position: (0, 11),
        }
        .run();
    }

    #[test]
    fn maintain_column_down() {
        TestCase {
            initial_text: "Long line\nShort\nLong line",
            initial_cursor: 8,
            expected_initial_text_position: (8, 0),

            keys: vec![key_event!('j'); 2],

            expected_text: "Long line\nShort\nLong line",
            expected_cursor: 24,
            expected_text_position: (8, 2),
        }
        .run();
    }

    #[test]
    fn maintain_column_up() {
        TestCase {
            initial_text: "Long line\nShort\nLong line",
            initial_cursor: 24,
            expected_initial_text_position: (8, 2),

            keys: vec![key_event!('k'); 2],

            expected_text: "Long line\nShort\nLong line",
            expected_cursor: 8,
            expected_text_position: (8, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start() {
        TestCase {
            initial_text: "Test text",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('w')],

            expected_text: "Test text",
            expected_cursor: 5,
            expected_text_position: (5, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_emoji() {
        TestCase {
            initial_text: "😀 hello",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('w')],

            expected_text: "😀 hello",
            expected_cursor: 5,
            expected_text_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_multiple() {
        TestCase {
            initial_text: "hello world test",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('w'); 3],

            expected_text: "hello world test",
            expected_cursor: 15,
            expected_text_position: (15, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_mid_word() {
        TestCase {
            initial_text: "hello world",
            initial_cursor: 1,
            expected_initial_text_position: (1, 0),

            keys: vec![key_event!('w')],

            expected_text: "hello world",
            expected_cursor: 6,
            expected_text_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_at_space() {
        TestCase {
            initial_text: "hello world",
            initial_cursor: 5,
            expected_initial_text_position: (5, 0),

            keys: vec![key_event!('w')],

            expected_text: "hello world",
            expected_cursor: 6,
            expected_text_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_end_of_file() {
        TestCase {
            initial_text: "hello world",
            initial_cursor: 6,
            expected_initial_text_position: (6, 0),

            keys: vec![key_event!('w')],

            expected_text: "hello world",
            expected_cursor: 10,
            expected_text_position: (10, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_punctuation_to_word() {
        TestCase {
            initial_text: "hello, world",
            initial_cursor: 5,
            expected_initial_text_position: (5, 0),

            keys: vec![key_event!('w')],

            expected_text: "hello, world",
            expected_cursor: 7,
            expected_text_position: (7, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_multiple_spaces() {
        TestCase {
            initial_text: "hello    world",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('w')],

            expected_text: "hello    world",
            expected_cursor: 9,
            expected_text_position: (9, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_with_numbers() {
        TestCase {
            initial_text: "test123 abc456",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('w')],

            expected_text: "test123 abc456",
            expected_cursor: 8,
            expected_text_position: (8, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_ignore_underscore() {
        TestCase {
            initial_text: "hello_world test",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('w')],

            expected_text: "hello_world test",
            expected_cursor: 12,
            expected_text_position: (12, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_across_lines() {
        TestCase {
            initial_text: "hello\nworld",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('w')],

            expected_text: "hello\nworld",
            expected_cursor: 6,
            expected_text_position: (0, 1),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_empty_file() {
        TestCase {
            initial_text: "",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('w')],

            expected_text: "",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_word_start_n() {
        TestCase {
            initial_text: "hello world test",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('3'), key_event!('w')],

            expected_text: "hello world test",
            expected_cursor: 15,
            expected_text_position: (15, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start() {
        TestCase {
            initial_text: "Test text",
            initial_cursor: 5,
            expected_initial_text_position: (5, 0),

            keys: vec![key_event!('b')],

            expected_text: "Test text",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_emoji() {
        TestCase {
            initial_text: "😀 hello",
            initial_cursor: 5,
            expected_initial_text_position: (3, 0),

            keys: vec![key_event!('b')],

            expected_text: "😀 hello",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_multiple() {
        TestCase {
            initial_text: "hello world test",
            initial_cursor: 15,
            expected_initial_text_position: (15, 0),

            keys: vec![key_event!('b'); 2],

            expected_text: "hello world test",
            expected_cursor: 6,
            expected_text_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_mid_word() {
        TestCase {
            initial_text: "hello world",
            initial_cursor: 8,
            expected_initial_text_position: (8, 0),

            keys: vec![key_event!('b')],

            expected_text: "hello world",
            expected_cursor: 6,
            expected_text_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_at_space() {
        TestCase {
            initial_text: "hello world ",
            initial_cursor: 11,
            expected_initial_text_position: (11, 0),

            keys: vec![key_event!('b')],

            expected_text: "hello world ",
            expected_cursor: 6,
            expected_text_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_start_of_file() {
        TestCase {
            initial_text: "hello world",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('b')],

            expected_text: "hello world",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_punctuation_to_word() {
        TestCase {
            initial_text: "hello world  ,",
            initial_cursor: 13,
            expected_initial_text_position: (13, 0),

            keys: vec![key_event!('b')],

            expected_text: "hello world  ,",
            expected_cursor: 6,
            expected_text_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_with_numbers() {
        TestCase {
            initial_text: "test123 abc456",
            initial_cursor: 8,
            expected_initial_text_position: (8, 0),

            keys: vec![key_event!('b')],

            expected_text: "test123 abc456",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_ignore_underscore() {
        TestCase {
            initial_text: "hello_world test",
            initial_cursor: 12,
            expected_initial_text_position: (12, 0),

            keys: vec![key_event!('b')],

            expected_text: "hello_world test",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_across_lines() {
        TestCase {
            initial_text: "hello\nworld",
            initial_cursor: 6,
            expected_initial_text_position: (0, 1),

            keys: vec![key_event!('b')],

            expected_text: "hello\nworld",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_empty_file() {
        TestCase {
            initial_text: "",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('b')],

            expected_text: "",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_word_start_n() {
        TestCase {
            initial_text: "hello world test",
            initial_cursor: 15,
            expected_initial_text_position: (15, 0),

            keys: vec![key_event!('2'), key_event!('b')],

            expected_text: "hello world test",
            expected_cursor: 6,
            expected_text_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn insert() {
        TestCase {
            initial_text: "lo",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![
                key_event!('i'),
                key_event!('H'),
                key_event!('e'),
                key_event!('l'),
            ],

            expected_text: "Hello",
            expected_cursor: 3,
            expected_text_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn delete_simple() {
        TestCase {
            initial_text: "Hello!",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![key_event!('i'), key_event!(Backspace)],

            expected_text: "Hllo!",
            expected_cursor: 1,
            expected_text_position: (1, 0),
        }
        .run();
    }

    #[test]
    fn delete_emoji() {
        TestCase {
            initial_text: "Hello ⚒️ !!",
            initial_cursor: 12,
            expected_initial_text_position: (8, 0),

            keys: vec![key_event!('i'), key_event!(Backspace)],

            expected_text: "Hello  !!",
            expected_cursor: 6,
            expected_text_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn delete_and_insert() {
        TestCase {
            initial_text: "Hello!!",
            initial_cursor: 6,
            expected_initial_text_position: (6, 0),

            keys: vec![
                key_event!('i'),
                key_event!(Backspace),
                key_event!(Backspace),
                key_event!(Backspace),
                key_event!(Backspace),
                key_event!('y'),
            ],
            expected_text: "Hey!",
            expected_cursor: 3,
            expected_text_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn normal_mode() {
        TestCase {
            initial_text: "Hello!",
            initial_cursor: 5,
            expected_initial_text_position: (5, 0),

            keys: vec![
                key_event!('i'),
                key_event!('!'),
                key_event!(Esc),
                key_event!('h'),
                key_event!('h'),
            ],
            expected_text: "Hello!!",
            expected_cursor: 4,
            expected_text_position: (4, 0),
        }
        .run();
    }

    #[test]
    fn insert_newline() {
        TestCase {
            initial_text: "Hello!",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![key_event!('i'), key_event!(Enter)],
            expected_text: "He\nllo!",
            expected_cursor: 3,
            expected_text_position: (0, 1),
        }
        .run();
    }

    #[test]
    fn move_cursor_line_end() {
        TestCase {
            initial_text: "Hello!!",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('$')],

            expected_text: "Hello!!",
            expected_cursor: 6,
            expected_text_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_line_end_with_lf() {
        TestCase {
            initial_text: "Hello!!\nNext line",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('$')],

            expected_text: "Hello!!\nNext line",
            expected_cursor: 6,
            expected_text_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_line_end_with_crlf() {
        TestCase {
            initial_text: "Hello!!\r\nNext line",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('$')],

            expected_text: "Hello!!\r\nNext line",
            expected_cursor: 6,
            expected_text_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_line_start() {
        TestCase {
            initial_text: "Hello!!",
            initial_cursor: 3,
            expected_initial_text_position: (3, 0),

            keys: vec![key_event!('0')],

            expected_text: "Hello!!",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_first_non_blank() {
        TestCase {
            initial_text: "   Hello!!",
            initial_cursor: 7,
            expected_initial_text_position: (7, 0),

            keys: vec![key_event!('^')],

            expected_text: "   Hello!!",
            expected_cursor: 3,
            expected_text_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_first_non_blank_empty() {
        TestCase {
            initial_text: "\nHello",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('^')],

            expected_text: "\nHello",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_paragraph() {
        TestCase {
            initial_text: "hello\nworld\n\nparagraph",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('}')],

            expected_text: "hello\nworld\n\nparagraph",
            expected_cursor: 12,
            expected_text_position: (0, 2),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_paragraph_consecutive_empty_lines() {
        TestCase {
            initial_text: "hello\nworld\n\n\n\n\nparagraph\n\n",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('}'); 2],

            expected_text: "hello\nworld\n\n\n\n\nparagraph\n\n",
            expected_cursor: 26,
            expected_text_position: (0, 7),
        }
        .run();
    }

    #[test]
    fn move_cursor_next_paragraph_n() {
        TestCase {
            initial_text: "hello\nworld\n\n\n\n\nparagraph\n\n",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('2'), key_event!('}')],

            expected_text: "hello\nworld\n\n\n\n\nparagraph\n\n",
            expected_cursor: 26,
            expected_text_position: (0, 7),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_paragraph() {
        TestCase {
            initial_text: "hello\n\nworld\n\n",
            initial_cursor: 13,
            expected_initial_text_position: (0, 3),

            keys: vec![key_event!('{')],

            expected_text: "hello\n\nworld\n\n",
            expected_cursor: 6,
            expected_text_position: (0, 1),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_paragraph_consecutive_empty_lines() {
        TestCase {
            initial_text: "hello\nworld\n\n\n\n\nparagraph\n\n",
            initial_cursor: 26,
            expected_initial_text_position: (0, 7),

            keys: vec![key_event!('{'); 2],

            expected_text: "hello\nworld\n\n\n\n\nparagraph\n\n",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_prev_paragraph_n() {
        TestCase {
            initial_text: "hello\nworld\n\n\n\n\nparagraph\n\n",
            initial_cursor: 26,
            expected_initial_text_position: (0, 7),

            keys: vec![key_event!('2'), key_event!('{')],

            expected_text: "hello\nworld\n\n\n\n\nparagraph\n\n",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn go_to_nth_or_last_line_no_n() {
        TestCase {
            initial_text: "hello\nworld\n",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('G')],

            expected_text: "hello\nworld\n",
            expected_cursor: 6,
            expected_text_position: (0, 1),
        }
        .run();
    }

    #[test]
    fn go_to_nth_or_last_line_with_n() {
        TestCase {
            initial_text: "hello\nworld\nyo\n",
            initial_cursor: 6,
            expected_initial_text_position: (0, 1),

            keys: vec![key_event!('1'), key_event!('G')],

            expected_text: "hello\nworld\nyo\n",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn go_to_last_line() {
        TestCase {
            initial_text: "hello\nworld\n",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('g'), key_event!('e')],

            expected_text: "hello\nworld\n",
            expected_cursor: 6,
            expected_text_position: (0, 1),
        }
        .run();
    }

    #[test]
    fn go_to_nth_or_first_line_no_n() {
        TestCase {
            initial_text: "hello\nworld\n",
            initial_cursor: 6,
            expected_initial_text_position: (0, 1),

            keys: vec![key_event!('g'), key_event!('g')],

            expected_text: "hello\nworld\n",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn go_to_nth_or_first_line_with_n() {
        TestCase {
            initial_text: "hello\nworld\nyo\n",
            initial_cursor: 6,
            expected_initial_text_position: (0, 1),

            keys: vec![key_event!('3'), key_event!('g'), key_event!('g')],

            expected_text: "hello\nworld\nyo\n",
            expected_cursor: 12,
            expected_text_position: (0, 2),
        }
        .run();
    }

    #[test]
    fn go_to_nth_or_first_line_with_middle_n() {
        TestCase {
            initial_text: "one\ntwo\nthree\nfour",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('2'), key_event!('g'), key_event!('g')],

            expected_text: "one\ntwo\nthree\nfour",
            expected_cursor: 4,
            expected_text_position: (0, 1),
        }
        .run();
    }

    #[test]
    fn delete_word_from_start() {
        TestCase {
            initial_text: "Hello world",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('d'), key_event!('w')],

            expected_text: "world",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn delete_word_from_middle() {
        TestCase {
            initial_text: "Hello world",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![key_event!('d'), key_event!('w')],

            expected_text: "Heworld",
            expected_cursor: 2,
            expected_text_position: (2, 0),
        }
        .run();
    }

    #[test]
    fn delete_word_stop_at_hyphen() {
        TestCase {
            initial_text: "Hello-world",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('d'), key_event!('w')],

            expected_text: "-world",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn delete_word_leading_whitespace() {
        TestCase {
            initial_text: "      Hello-world",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('d'), key_event!('w')],

            expected_text: "Hello-world",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn delete_word_n() {
        TestCase {
            initial_text: "Hello world several words",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('2'), key_event!('d'), key_event!('w')],

            expected_text: "several words",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn change_word_from_start() {
        TestCase {
            initial_text: "Hello world",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![
                key_event!('c'),
                key_event!('w'),
                key_event!('h'),
                key_event!('i'),
            ],

            expected_text: "hiworld",
            expected_cursor: 2,
            expected_text_position: (2, 0),
        }
        .run();
    }

    #[test]
    fn change_word_from_middle() {
        TestCase {
            initial_text: "Hello world",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![
                key_event!('c'),
                key_event!('w'),
                key_event!('h'),
                key_event!('i'),
            ],

            expected_text: "Hehiworld",
            expected_cursor: 4,
            expected_text_position: (4, 0),
        }
        .run();
    }

    #[test]
    fn change_word_stop_at_hyphen() {
        TestCase {
            initial_text: "Hello-world",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![
                key_event!('c'),
                key_event!('w'),
                key_event!('h'),
                key_event!('i'),
            ],

            expected_text: "hi-world",
            expected_cursor: 2,
            expected_text_position: (2, 0),
        }
        .run();
    }

    #[test]
    fn change_word_leading_whitespace() {
        TestCase {
            initial_text: "      Hello-world",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![
                key_event!('c'),
                key_event!('w'),
                key_event!('h'),
                key_event!('i'),
            ],

            expected_text: "hiHello-world",
            expected_cursor: 2,
            expected_text_position: (2, 0),
        }
        .run();
    }

    #[test]
    fn change_word_n() {
        TestCase {
            initial_text: "Hello world several words",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![
                key_event!('2'),
                key_event!('c'),
                key_event!('w'),
                key_event!('H'),
                key_event!('i'),
                key_event!(' '),
            ],

            expected_text: "Hi several words",
            expected_cursor: 3,
            expected_text_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn delete_to_line_end() {
        TestCase {
            initial_text: "Hello there!\nNext line",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![key_event!('d'), key_event!('$')],

            expected_text: "He\nNext line",
            expected_cursor: 2,
            expected_text_position: (2, 0),
        }
        .run();
    }

    #[test]
    fn change_to_line_end() {
        TestCase {
            initial_text: "Hello there!\nNext line",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![
                key_event!('c'),
                key_event!('$'),
                key_event!('y'),
                key_event!('!'),
            ],

            expected_text: "Hey!\nNext line",
            expected_cursor: 4,
            expected_text_position: (4, 0),
        }
        .run();
    }

    #[test]
    fn delete_to_line_start() {
        TestCase {
            initial_text: "Hello there!\n     Next line!",
            initial_cursor: 26,
            expected_initial_text_position: (13, 1),

            keys: vec![key_event!('d'), key_event!('0')],

            expected_text: "Hello there!\n!",
            expected_cursor: 13,
            expected_text_position: (0, 1),
        }
        .run();
    }

    #[test]
    fn delete_to_line_first_non_blank() {
        TestCase {
            initial_text: "Hello there!\n     Next line!",
            initial_cursor: 26,
            expected_initial_text_position: (13, 1),

            keys: vec![key_event!('d'), key_event!('^')],

            expected_text: "Hello there!\n     !",
            expected_cursor: 18,
            expected_text_position: (5, 1),
        }
        .run();
    }

    #[test]
    fn delete_line() {
        TestCase {
            initial_text: "Hello there!\nNext line!",
            initial_cursor: 4,
            expected_initial_text_position: (4, 0),

            keys: vec![key_event!('d'), key_event!('d')],

            expected_text: "Next line!",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn delete_line_n() {
        TestCase {
            initial_text: "Hello there!\nNext line!\nAnd another line!",
            initial_cursor: 4,
            expected_initial_text_position: (4, 0),

            keys: vec![key_event!('2'), key_event!('d'), key_event!('d')],

            expected_text: "And another line!",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn delete_whole_word() {
        TestCase {
            initial_text: "Hello there!!!",
            initial_cursor: 8,
            expected_initial_text_position: (8, 0),

            keys: vec![key_event!('d'), key_event!('i'), key_event!('w')],

            expected_text: "Hello !!!",
            expected_cursor: 6,
            expected_text_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn delete_whole_word_from_start() {
        TestCase {
            initial_text: "Hello there!!!",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('d'), key_event!('i'), key_event!('w')],

            expected_text: " there!!!",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn delete_whole_word_from_end() {
        TestCase {
            initial_text: "Hello there!!!",
            initial_cursor: 4,
            expected_initial_text_position: (4, 0),

            keys: vec![key_event!('d'), key_event!('i'), key_event!('w')],

            expected_text: " there!!!",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn delete_to_prev_word_start() {
        TestCase {
            initial_text: "Hello there!!!",
            initial_cursor: 6,
            expected_initial_text_position: (6, 0),

            keys: vec![key_event!('d'), key_event!('b')],

            expected_text: "there!!!",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn delete_to_prev_word_start_n() {
        TestCase {
            initial_text: "Hello there words words words!!!",
            initial_cursor: 26,
            expected_initial_text_position: (26, 0),

            keys: vec![key_event!('4'), key_event!('d'), key_event!('b')],

            expected_text: "Hello rds!!!",
            expected_cursor: 6,
            expected_text_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn append_text() {
        TestCase {
            initial_text: "Hello",
            initial_cursor: 1,
            expected_initial_text_position: (1, 0),

            keys: vec![key_event!('a'), key_event!('y'), key_event!('y')],

            expected_text: "Heyyllo",
            expected_cursor: 4,
            expected_text_position: (4, 0),
        }
        .run();
    }

    #[test]
    fn append_text_end_of_line_no_newline() {
        TestCase {
            initial_text: "Hello",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![key_event!('A'), key_event!('!'), key_event!('!')],

            expected_text: "Hello!!\n",
            expected_cursor: 7,
            expected_text_position: (7, 0),
        }
        .run();
    }

    #[test]
    fn append_text_end_of_line_with_newline() {
        TestCase {
            initial_text: "Hello\n",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![key_event!('A'), key_event!('!'), key_event!('!')],

            expected_text: "Hello!!\n",
            expected_cursor: 7,
            expected_text_position: (7, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_word_end() {
        TestCase {
            initial_text: "Hello__123: there",
            initial_cursor: 1,
            expected_initial_text_position: (1, 0),

            keys: vec![key_event!('e')],

            expected_text: "Hello__123: there",
            expected_cursor: 9,
            expected_text_position: (9, 0),
        }
        .run();
    }

    #[test]
    fn move_cursor_word_end_n() {
        TestCase {
            initial_text: "Hello__123: there words words words",
            initial_cursor: 1,
            expected_initial_text_position: (1, 0),

            keys: vec![key_event!('4'), key_event!('e')],

            expected_text: "Hello__123: there words words words",
            expected_cursor: 22,
            expected_text_position: (22, 0),
        }
        .run();
    }

    #[test]
    fn delete_to_word_end() {
        TestCase {
            initial_text: "Hello there",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![key_event!('d'), key_event!('e')],

            expected_text: "He there",
            expected_cursor: 2,
            expected_text_position: (2, 0),
        }
        .run();
    }

    #[test]
    fn delete_to_word_end_n() {
        TestCase {
            initial_text: "Hello there there there there there",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![key_event!('4'), key_event!('d'), key_event!('e')],

            expected_text: "He there there",
            expected_cursor: 2,
            expected_text_position: (2, 0),
        }
        .run();
    }

    #[test]
    fn change_to_line_start() {
        TestCase {
            initial_text: "Hello there!\n     Next line!",
            initial_cursor: 26,
            expected_initial_text_position: (13, 1),

            keys: vec![
                key_event!('c'),
                key_event!('0'),
                key_event!('y'),
                key_event!('o'),
                key_event!('!'),
            ],

            expected_text: "Hello there!\nyo!!",
            expected_cursor: 16,
            expected_text_position: (3, 1),
        }
        .run();
    }

    #[test]
    fn change_to_line_first_non_blank() {
        TestCase {
            initial_text: "Hello there!\n     Next line!",
            initial_cursor: 26,
            expected_initial_text_position: (13, 1),

            keys: vec![
                key_event!('c'),
                key_event!('^'),
                key_event!('y'),
                key_event!('o'),
                key_event!('!'),
            ],

            expected_text: "Hello there!\n     yo!!",
            expected_cursor: 21,
            expected_text_position: (8, 1),
        }
        .run();
    }

    #[test]
    fn change_line() {
        TestCase {
            initial_text: "Hello there!\nNext line!",
            initial_cursor: 4,
            expected_initial_text_position: (4, 0),

            keys: vec![
                key_event!('c'),
                key_event!('c'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
            ],

            expected_text: "Hey\nNext line!",
            expected_cursor: 3,
            expected_text_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn change_line_no_newline() {
        TestCase {
            initial_text: "Hello there!",
            initial_cursor: 4,
            expected_initial_text_position: (4, 0),

            keys: vec![
                key_event!('c'),
                key_event!('c'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
            ],

            expected_text: "Hey\n",
            expected_cursor: 3,
            expected_text_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn change_line_n() {
        TestCase {
            initial_text: "Hello there!\nNext line!\nAnother line!",
            initial_cursor: 4,
            expected_initial_text_position: (4, 0),

            keys: vec![
                key_event!('2'),
                key_event!('c'),
                key_event!('c'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
            ],

            expected_text: "Hey\nAnother line!",
            expected_cursor: 3,
            expected_text_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn change_whole_word() {
        TestCase {
            initial_text: "Hello there!!!",
            initial_cursor: 8,
            expected_initial_text_position: (8, 0),

            keys: vec![
                key_event!('c'),
                key_event!('i'),
                key_event!('w'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
            ],

            expected_text: "Hello Hey!!!",
            expected_cursor: 9,
            expected_text_position: (9, 0),
        }
        .run();
    }

    #[test]
    fn change_whole_word_from_start() {
        TestCase {
            initial_text: "Hello there!!!",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![
                key_event!('c'),
                key_event!('i'),
                key_event!('w'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
            ],

            expected_text: "Hey there!!!",
            expected_cursor: 3,
            expected_text_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn change_whole_word_from_end() {
        TestCase {
            initial_text: "Hello there!!!",
            initial_cursor: 4,
            expected_initial_text_position: (4, 0),

            keys: vec![
                key_event!('c'),
                key_event!('i'),
                key_event!('w'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
            ],

            expected_text: "Hey there!!!",
            expected_cursor: 3,
            expected_text_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn change_to_prev_word_start() {
        TestCase {
            initial_text: "Hello there!!!",
            initial_cursor: 8,
            expected_initial_text_position: (8, 0),

            keys: vec![key_event!('c'), key_event!('b'), key_event!('H')],

            expected_text: "Hello Here!!!",
            expected_cursor: 7,
            expected_text_position: (7, 0),
        }
        .run();
    }

    #[test]
    fn change_to_prev_word_start_from_word_start() {
        TestCase {
            initial_text: "Hello there!!!",
            initial_cursor: 6,
            expected_initial_text_position: (6, 0),

            keys: vec![
                key_event!('c'),
                key_event!('b'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
                key_event!(' '),
            ],

            expected_text: "Hey there!!!",
            expected_cursor: 4,
            expected_text_position: (4, 0),
        }
        .run();
    }

    #[test]
    fn change_to_prev_word_start_n() {
        TestCase {
            initial_text: "Hello there!!!",
            initial_cursor: 8,
            expected_initial_text_position: (8, 0),

            keys: vec![
                key_event!('2'),
                key_event!('c'),
                key_event!('b'),
                key_event!('H'),
            ],

            expected_text: "Here!!!",
            expected_cursor: 1,
            expected_text_position: (1, 0),
        }
        .run();
    }

    #[test]
    fn change_to_word_end() {
        TestCase {
            initial_text: "Hello there",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![key_event!('c'), key_event!('e'), key_event!('y')],

            expected_text: "Hey there",
            expected_cursor: 3,
            expected_text_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn change_to_word_end_n() {
        TestCase {
            initial_text: "Hello there words words",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![
                key_event!('3'),
                key_event!('c'),
                key_event!('e'),
                key_event!('y'),
            ],

            expected_text: "Hey words",
            expected_cursor: 3,
            expected_text_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn visual_mode_delete() {
        TestCase {
            initial_text: "Hello there\nAnother line!",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![
                key_event!('v'),
                key_event!('j'),
                key_event!('l'),
                key_event!('d'),
            ],

            expected_text: "Heher line!",
            expected_cursor: 2,
            expected_text_position: (2, 0),
        }
        .run();
    }

    #[test]
    fn visual_mode_change() {
        TestCase {
            initial_text: "Hello there\nAnother line!",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![
                key_event!('v'),
                key_event!('j'),
                key_event!('l'),
                key_event!('c'),
                key_event!('y'),
                key_event!(Enter),
                key_event!('a'),
                key_event!('n'),
                key_event!('o'),
                key_event!('t'),
            ],

            expected_text: "Hey\nanother line!",
            expected_cursor: 8,
            expected_text_position: (4, 1),
        }
        .run();
    }

    #[test]
    fn reverse_selection() {
        TestCase {
            initial_text: "Hello there\nAnother line!",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![
                key_event!('v'),
                key_event!('j'),
                key_event!('l'),
                key_event!('o'),
            ],

            expected_text: "Hello there\nAnother line!",
            expected_cursor: 2,
            expected_text_position: (2, 0),
        }
        .run();
    }

    #[test]
    fn reverse_selection_twice() {
        TestCase {
            initial_text: "Hello there\nAnother line!",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![
                key_event!('v'),
                key_event!('j'),
                key_event!('l'),
                key_event!('o'),
                key_event!('o'),
            ],

            expected_text: "Hello there\nAnother line!",
            expected_cursor: 15,
            expected_text_position: (3, 1),
        }
        .run();
    }

    #[test]
    fn open_new_line() {
        TestCase {
            initial_text: "Hello there\nAnother line!",
            initial_cursor: 15,
            expected_initial_text_position: (3, 1),

            keys: vec![
                key_event!('o'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
            ],

            expected_text: "Hello there\nAnother line!\nHey\n",
            expected_cursor: 29,
            expected_text_position: (3, 2),
        }
        .run();
    }

    #[test]
    fn open_new_line_middle() {
        TestCase {
            initial_text: "Hello there\nAnother line!\nAgain a line :)",
            initial_cursor: 15,
            expected_initial_text_position: (3, 1),

            keys: vec![
                key_event!('o'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
            ],

            expected_text: "Hello there\nAnother line!\nHey\nAgain a line :)",
            expected_cursor: 29,
            expected_text_position: (3, 2),
        }
        .run();
    }

    #[test]
    fn open_new_line_above() {
        TestCase {
            initial_text: "Hello there\nAnother line!\nAgain a line :)",
            initial_cursor: 15,
            expected_initial_text_position: (3, 1),

            keys: vec![
                key_event!('O'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
            ],

            expected_text: "Hello there\nHey\nAnother line!\nAgain a line :)",
            expected_cursor: 15,
            expected_text_position: (3, 1),
        }
        .run();
    }

    #[test]
    fn open_new_line_above_top() {
        TestCase {
            initial_text: "Hello there\nAnother line!\nAgain a line :)",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![
                key_event!('O'),
                key_event!('H'),
                key_event!('e'),
                key_event!('y'),
            ],

            expected_text: "Hey\nHello there\nAnother line!\nAgain a line :)",
            expected_cursor: 3,
            expected_text_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn select_current_word() {
        TestCase {
            initial_text: "Hello there",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![
                key_event!('v'),
                key_event!('i'),
                key_event!('w'),
                key_event!('d'),
            ],

            expected_text: " there",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn delete_down() {
        TestCase {
            initial_text: "Hello there\nAnother line\n     And another",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![key_event!('d'), key_event!('j')],

            expected_text: "     And another",
            expected_cursor: 5,
            expected_text_position: (5, 0),
        }
        .run();
    }

    #[test]
    fn delete_down_n() {
        TestCase {
            initial_text: "Hello there\nAnother line\n     And another\nAnd one more",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![key_event!('2'), key_event!('d'), key_event!('j')],

            expected_text: "And one more",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn delete_up() {
        TestCase {
            initial_text: "Hello there\nAnother line\nAnd another",
            initial_cursor: 27,
            expected_initial_text_position: (2, 2),

            keys: vec![key_event!('d'), key_event!('k')],

            expected_text: "Hello there\n",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn delete_up_n() {
        TestCase {
            initial_text: "Hello there\nAnother line\nAnd another\nAnd one more",
            initial_cursor: 27,
            expected_initial_text_position: (2, 2),

            keys: vec![key_event!('2'), key_event!('d'), key_event!('k')],

            expected_text: "And one more",
            expected_cursor: 0,
            expected_text_position: (0, 0),
        }
        .run();
    }

    #[test]
    fn clear_key_sequence_with_esc() {
        TestCase {
            initial_text: "Hello there\nAnother line\nAnd another\nAnd one more",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('2'), key_event!(Esc), key_event!('j')],

            expected_text: "Hello there\nAnother line\nAnd another\nAnd one more",
            expected_cursor: 12,
            expected_text_position: (0, 1),
        }
        .run();
    }

    #[test]
    fn match_pairs_braces() {
        TestCase {
            initial_text: "{Hello }",
            initial_cursor: 0,
            expected_initial_text_position: (0, 0),

            keys: vec![key_event!('%')],

            expected_text: "{Hello }",
            expected_cursor: 7,
            expected_text_position: (7, 0),
        }
        .run();
    }

    #[test]
    fn match_pairs_brackets() {
        TestCase {
            initial_text: "H[ello]",
            initial_cursor: 1,
            expected_initial_text_position: (1, 0),

            keys: vec![key_event!('%')],

            expected_text: "H[ello]",
            expected_cursor: 6,
            expected_text_position: (6, 0),
        }
        .run();
    }

    #[test]
    fn match_pairs_parens() {
        TestCase {
            initial_text: "He()llo",
            initial_cursor: 2,
            expected_initial_text_position: (2, 0),

            keys: vec![key_event!('%')],

            expected_text: "He()llo",
            expected_cursor: 3,
            expected_text_position: (3, 0),
        }
        .run();
    }

    #[test]
    fn match_pairs_after_line_end() {
        TestCase {
            initial_text: "hello {\nworld } test",
            initial_cursor: 1,
            expected_initial_text_position: (1, 0),

            keys: vec![key_event!('$'), key_event!('%')],

            expected_text: "hello {\nworld } test",
            expected_cursor: 14,
            expected_text_position: (6, 1),
        }
        .run();
    }
}

#[cfg(test)]
mod proptests {
    use std::io::Write as _;

    use proptest::prelude::*;

    use super::*;

    fn strip_trailing_line_break(line: &str) -> &str {
        line.strip_suffix("\r\n")
            .or_else(|| line.strip_suffix('\n'))
            .unwrap_or(line)
    }

    fn chars_no_line_break() -> impl Strategy<Value = char> {
        any::<char>().prop_filter("not a line break", |&ch| ch != '\n' && ch != '\r')
    }

    fn line_content_strategy(range: Range<usize>) -> impl Strategy<Value = String> {
        prop::collection::vec(chars_no_line_break(), range)
            .prop_map(|chars| chars.into_iter().collect())
    }

    fn terminated_line_strategy() -> impl Strategy<Value = String> {
        (line_content_strategy(0..80), prop_oneof![
            Just("\n"),
            Just("\r\n")
        ])
            .prop_map(|(content, ending)| content + ending)
    }

    fn current_line_strategy() -> impl Strategy<Value = String> {
        prop_oneof![
            // if there's no line ending, it needs to have length at least 1, because
            // otherwise the "line" will just be an empty string, which makes no sense
            line_content_strategy(1..80),
            (line_content_strategy(0..80), prop_oneof![
                Just("\n"),
                Just("\r\n")
            ])
                .prop_map(|(content, ending)| content + ending),
        ]
    }

    /// This is a reference implementation for finding the offset from the start
    /// of a given line.
    fn oracle_first_non_blank_offset(line: &str) -> usize {
        let mut offset = 0;

        for char in strip_trailing_line_break(line).chars() {
            if char.is_whitespace() {
                offset += char.len_utf8();
            } else {
                return offset;
            }
        }

        0
    }

    const TEST_DIMENSIONS: Dimensions = Dimensions::new(Columns::new(80), Rows::new(24));

    fn doc(contents: &str) -> Document {
        let mut temp_file = tempfile::NamedTempFile::new().unwrap();
        write!(temp_file, "{contents}").unwrap();

        Document::new(temp_file.path().to_path_buf(), TEST_DIMENSIONS).unwrap()
    }

    proptest! {
        #[test]
        fn move_cursor_first_non_blank(
            prefix in prop::collection::vec(terminated_line_strategy(), 0..10),
            current in current_line_strategy(),
            postfix in prop::collection::vec(terminated_line_strategy(), 0..10)
        ) {
            let prefix_text = prefix.concat();
            let initial_cursor = prefix_text.len();

            let postfix_text = if current.ends_with('\n') {
                postfix.concat()
            } else {
                String::new()
            };

            let text = format!("{prefix_text}{current}{postfix_text}");
            let mut document = doc(&text);
            document.set_cursor(ByteIndex::new(initial_cursor));

            let _ = document.handle_key_event(KeyEvent::from(KeyCode::Char('^')), &mut EventContext::new());

            prop_assert!(document.text.is_char_boundary(document.selection.cursor.value()));

            let expected = ByteIndex::new(initial_cursor + oracle_first_non_blank_offset(&current));
            prop_assert_eq!(document.selection.cursor, expected);

        }
    }
}
