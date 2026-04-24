use std::{
    io,
    panic,
    sync,
    time::Duration,
};

use crossterm::{
    event::{
        self,
        Event,
        KeyCode,
    },
    style,
    terminal::{
        BeginSynchronizedUpdate,
        EndSynchronizedUpdate,
    },
};

use crate::{
    args::Args,
    buffer::Buffer,
    document::Document,
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
        let mut document = Document::new(args.file_path, self.buffer.dimensions())?;

        let (jj_desc_tx, jj_desc_rx) = sync::mpsc::channel();
        document.poll_jj(jj_desc_tx);

        document.render(&mut self.buffer);
        self.draw(document.visual_cursor_position())?;

        loop {
            self.buffer.clear();

            if event::poll(Duration::from_millis(16))? {
                let event_outcome = match event::read()? {
                    Event::Key(key_event) => {
                        if key_event.code == KeyCode::Char('q') {
                            break;
                        }

                        Some(document.handle_key_event(key_event))
                    }
                    Event::Mouse(_mouse_event) => todo!(),
                    Event::Resize(columns, rows) => {
                        let dimensions = Dimensions::new(Columns::from(columns), Rows::from(rows));
                        self.buffer = Buffer::new(dimensions);
                        document.resize(dimensions);

                        Some(EventOutcome::Handled)
                    }

                    Event::FocusGained | Event::FocusLost | Event::Paste(_) => todo!(),
                };

                match event_outcome {
                    Some(EventOutcome::Handled) => document.render(&mut self.buffer),
                    Some(EventOutcome::Unhandled) => todo!(),
                    None => todo!(),
                }

                self.draw(document.visual_cursor_position())?;
            }

            if let Ok(desc_result) = jj_desc_rx.try_recv()
                && let Ok(desc) = desc_result
            {
                document.set_jj_change_description(desc);
                document.render(&mut self.buffer);
                self.draw(document.visual_cursor_position())?;
            }
        }

        Ok(())
    }

    fn draw(&mut self, cursor_position: Position) -> io::Result<()> {
        crossterm::execute!(self.out, BeginSynchronizedUpdate)?;

        crossterm::queue!(
            self.out,
            crossterm::cursor::Hide,
            crossterm::cursor::MoveTo(0, 0)
        )?;

        let cells = self.buffer.cells();
        let mut buffer_index = 0;

        while buffer_index < cells.len() {
            let cell = &cells[buffer_index];

            crossterm::queue!(
                self.out,
                style::SetForegroundColor(cell.foreground()),
                style::SetBackgroundColor(cell.background()),
                style::Print(cell.content())
            )?;

            buffer_index += cell.width();
        }

        crossterm::queue!(
            self.out,
            crossterm::cursor::MoveTo(
                u16::try_from(cursor_position.left().value())
                    .expect("cursor column should be <= u16::MAX"),
                u16::try_from(cursor_position.top().value())
                    .expect("cursor row should be <= u16::MAX"),
            ),
            crossterm::cursor::Show
        )?;

        crossterm::execute!(self.out, EndSynchronizedUpdate)?;

        Ok(())
    }
}

#[derive(Debug)]
#[must_use]
pub(crate) enum EventOutcome {
    Handled,
    Unhandled,
}
