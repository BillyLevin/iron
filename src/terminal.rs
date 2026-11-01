use std::{
    io,
    panic,
    path::Path,
    time::Duration,
};

use crossterm::{
    event::{
        self,
        Event,
    },
    style::{
        self,
        Attribute,
        Attributes,
        Color,
    },
    terminal::{
        BeginSynchronizedUpdate,
        EndSynchronizedUpdate,
    },
};

use crate::{
    args::Args,
    buffer::Buffer,
    editor::{
        Editor,
        EventOutcome,
    },
    file_picker::FileIndex,
    jujutsu::JJPoller,
    lsp::LspClient,
    style::StyleAttributes,
    ui::{
        AppEvent,
        Columns,
        Dimensions,
        EditorDimensions,
        LayerEvent,
        Position,
        Rows,
    },
};

pub struct Terminal {
    out: io::Stdout,
    buffer: Buffer,
}

impl Terminal {
    pub fn run(args: Args) -> anyhow::Result<()> {
        let (columns, rows) = crossterm::terminal::size()?;

        let mut terminal = Self {
            out: io::stdout(),
            buffer: Buffer::new(Dimensions::new(Columns::from(columns), Rows::from(rows))),
        };

        terminal.enter()?;

        let old_hook = panic::take_hook();
        panic::set_hook(Box::new(move |panic_info| {
            let mut stdout = io::stdout();
            let _ = crossterm::execute!(&mut stdout, crossterm::terminal::LeaveAlternateScreen);
            let _ = crossterm::terminal::disable_raw_mode();

            old_hook(panic_info);
        }));

        terminal.run_event_loop(args)?;

        terminal.exit()?;

        Ok(())
    }

    fn enter(&mut self) -> io::Result<()> {
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(self.out, crossterm::terminal::EnterAlternateScreen)?;

        Ok(())
    }

    fn exit(&mut self) -> io::Result<()> {
        crossterm::execute!(self.out, crossterm::terminal::LeaveAlternateScreen)?;
        crossterm::terminal::disable_raw_mode()?;

        Ok(())
    }

    fn run_event_loop(&mut self, args: Args) -> anyhow::Result<()> {
        let lsp = LspClient::new()
            .inspect_err(|err| log::error!("failed to start LSP client: {err}"))
            .ok();
        let jj_poller = JJPoller::new(args.file_path.clone());

        let mut editor = Editor::new(
            args.file_path,
            self.editor_dimensions().unwrap_or(EditorDimensions::MIN),
            lsp,
            jj_poller,
            FileIndex::new(Path::new(".")),
        )?;

        self.render(&mut editor)?;

        loop {
            puffin::GlobalProfiler::lock().new_frame();

            let mut rerender = false;

            if event::poll(Duration::from_millis(16))? {
                puffin::profile_scope!("terminal_event");

                let terminal_event = event::read()?;

                let app_event = if let Event::Resize(columns, rows) = terminal_event {
                    self.buffer
                        .resize(Dimensions::new(Columns::from(columns), Rows::from(rows)));

                    rerender = true;

                    self.editor_dimensions().map(AppEvent::Resize)
                } else {
                    self.editor_dimensions()
                        .and_then(|_| LayerEvent::from_terminal_event(terminal_event))
                        .map(AppEvent::Layer)
                };

                if let Some(app_event) = app_event {
                    match editor.handle_event(&app_event) {
                        EventOutcome::Handled => rerender = true,
                        EventOutcome::Unhandled => {}
                        EventOutcome::CloseApp => break,
                    }
                }
            }

            match editor.handle_layer_events() {
                EventOutcome::Handled => rerender = true,
                EventOutcome::Unhandled => {}
                EventOutcome::CloseApp => break,
            }

            if rerender {
                self.render(&mut editor)?;
            }
        }

        Ok(())
    }

    const fn editor_dimensions(&self) -> Option<EditorDimensions> {
        EditorDimensions::new(self.buffer.dimensions())
    }

    fn render(&mut self, editor: &mut Editor) -> io::Result<()> {
        self.buffer.clear();

        let is_too_small = self.editor_dimensions().is_none();

        if is_too_small {
            draw(&mut self.out, &self.buffer, Position::default())
        } else {
            editor.render(&mut self.buffer);
            draw(&mut self.out, &self.buffer, editor.visual_cursor_position())
        }
    }
}

pub(crate) fn draw(
    out: &mut impl io::Write,
    buffer: &Buffer,
    cursor_position: Position,
) -> io::Result<()> {
    crossterm::execute!(out, BeginSynchronizedUpdate)?;

    crossterm::queue!(out, crossterm::cursor::MoveTo(0, 0))?;

    let mut fg = Color::Reset;
    let mut bg = Color::Reset;
    let mut underline_color = Color::Reset;
    let mut attributes = StyleAttributes::empty();

    for cell in buffer.visual_cells().flatten() {
        if let new_fg = cell.foreground()
            && new_fg != fg
        {
            fg = new_fg;
            crossterm::queue!(out, style::SetForegroundColor(fg))?;
        }

        if let new_bg = cell.background()
            && new_bg != bg
        {
            bg = new_bg;
            crossterm::queue!(out, style::SetBackgroundColor(bg))?;
        }

        if let new_underline_color = cell.underline_color()
            && new_underline_color != underline_color
        {
            underline_color = new_underline_color;
            crossterm::queue!(out, style::SetUnderlineColor(underline_color))?;
        }

        if let new_attributes = cell.attributes()
            && new_attributes != attributes
        {
            let diff = attributes_diff(attributes, new_attributes);
            attributes = new_attributes;
            crossterm::queue!(out, style::SetAttributes(diff))?;
        }

        crossterm::queue!(out, style::Print(cell.content()))?;
    }

    crossterm::queue!(
        out,
        crossterm::cursor::MoveTo(
            u16::try_from(cursor_position.left().value())
                .expect("cursor column should be <= u16::MAX"),
            u16::try_from(cursor_position.top().value()).expect("cursor row should be <= u16::MAX"),
        ),
    )?;

    crossterm::execute!(out, EndSynchronizedUpdate)?;

    Ok(())
}

/// Calculates all of the [`Attributes`] that need to be queued to the terminal
/// based on the diff between the current and next [`StyleAttributes`].
fn attributes_diff(current: StyleAttributes, next: StyleAttributes) -> Attributes {
    let mut result = Attributes::none();

    let removed = current - next;
    for attribute in removed {
        let opposite = match attribute {
            StyleAttributes::Bold => Attribute::NormalIntensity,
            StyleAttributes::Italic => Attribute::NoItalic,
            StyleAttributes::Underlined | StyleAttributes::Undercurled => Attribute::NoUnderline,
            _ => unreachable!(),
        };

        result.set(opposite);
    }

    let added = next - current;
    for attribute in added {
        result.set(match attribute {
            StyleAttributes::Bold => Attribute::Bold,
            StyleAttributes::Italic => Attribute::Italic,
            StyleAttributes::Underlined => Attribute::Underlined,
            StyleAttributes::Undercurled => Attribute::Undercurled,
            _ => unreachable!(),
        });
    }

    result
}
