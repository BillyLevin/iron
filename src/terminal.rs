use std::{
    io,
    panic,
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
    style::StyleAttributes,
    ui::{
        Columns,
        Dimensions,
        Position,
        Rows,
    },
};

pub struct Terminal {
    out: io::Stdout,
    buffer: Buffer,
}

impl Terminal {
    pub fn run(args: Args) -> io::Result<()> {
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

    fn run_event_loop(&mut self, args: Args) -> io::Result<()> {
        let mut editor = Editor::new(args.file_path, self.buffer.dimensions())?;
        editor.render(&mut self.buffer);
        self.draw(editor.visual_cursor_position())?;

        loop {
            puffin::GlobalProfiler::lock().new_frame();

            self.buffer.clear();

            let mut rerender = false;

            if event::poll(Duration::from_millis(16))? {
                puffin::profile_scope!("terminal_event");

                let terminal_event = event::read()?;

                match terminal_event {
                    Event::Resize(columns, rows) => {
                        let dimensions = Dimensions::new(Columns::from(columns), Rows::from(rows));
                        self.buffer.resize(dimensions);
                        rerender = true;
                    }
                    Event::FocusGained
                    | Event::FocusLost
                    | Event::Key(_)
                    | Event::Mouse(_)
                    | Event::Paste(_) => {}
                }

                match editor.handle_event(&terminal_event) {
                    EventOutcome::Handled => rerender = true,
                    EventOutcome::Unhandled => {}
                    EventOutcome::CloseApp => break,
                }
            }

            match editor.handle_layer_events() {
                EventOutcome::Handled => rerender = true,
                EventOutcome::Unhandled => {}
                EventOutcome::CloseApp => break,
            }

            if rerender {
                editor.render(&mut self.buffer);
                self.draw(editor.visual_cursor_position())?;
            }
        }

        Ok(())
    }

    fn draw(&mut self, cursor_position: Position) -> io::Result<()> {
        crossterm::execute!(self.out, BeginSynchronizedUpdate)?;

        crossterm::queue!(self.out, crossterm::cursor::MoveTo(0, 0))?;

        let cells = self.buffer.cells();
        let mut buffer_index = 0;

        let mut fg = Color::Reset;
        let mut bg = Color::Reset;
        let mut attributes = StyleAttributes::empty();

        while buffer_index < cells.len() {
            let cell = &cells[buffer_index];

            if let new_fg = cell.foreground()
                && new_fg != fg
            {
                fg = new_fg;
                crossterm::queue!(self.out, style::SetForegroundColor(fg))?;
            }

            if let new_bg = cell.background()
                && new_bg != bg
            {
                bg = new_bg;
                crossterm::queue!(self.out, style::SetBackgroundColor(bg))?;
            }

            if let new_attributes = cell.attributes()
                && new_attributes != attributes
            {
                let diff = attributes_diff(attributes, new_attributes);
                attributes = new_attributes;
                crossterm::queue!(self.out, style::SetAttributes(diff))?;
            }

            crossterm::queue!(self.out, style::Print(cell.content()))?;

            buffer_index += cell.width().value();
        }

        crossterm::queue!(
            self.out,
            crossterm::cursor::MoveTo(
                u16::try_from(cursor_position.left().value())
                    .expect("cursor column should be <= u16::MAX"),
                u16::try_from(cursor_position.top().value())
                    .expect("cursor row should be <= u16::MAX"),
            ),
        )?;

        crossterm::execute!(self.out, EndSynchronizedUpdate)?;

        Ok(())
    }
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
            StyleAttributes::Underlined => Attribute::NoUnderline,
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
            _ => unreachable!(),
        });
    }

    result
}
