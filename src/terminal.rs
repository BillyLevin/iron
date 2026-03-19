use std::{
    io::{self, Write as _},
    ops, panic,
};

use crossterm::{
    event::{self, Event, KeyCode},
    style,
};

use crate::{
    args::Args,
    buffer::Buffer,
    document::{Document, Position},
};

pub struct Terminal {
    out: io::Stdout,
    dimensions: Dimensions,
}

impl Terminal {
    pub fn run(args: &Args) -> io::Result<()> {
        let (columns, rows) = crossterm::terminal::size()?;

        let mut terminal = Self {
            out: io::stdout(),
            dimensions: Dimensions::new(Columns::from(columns), Rows::from(rows)),
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

    fn run_event_loop(&mut self, args: &Args) -> io::Result<()> {
        let mut document = Document::new(&args.file_path, self.dimensions)?;
        let mut buffer = Buffer::new(self.dimensions);

        document.render(&mut buffer);
        self.draw(&buffer, document.visual_cursor_position())?;

        loop {
            buffer.clear();

            let event_outcome = match event::read()? {
                Event::Key(key_event) => {
                    if key_event.code == KeyCode::Char('q') {
                        break;
                    }

                    Some(document.handle_key_event(key_event))
                }
                Event::Mouse(_mouse_event) => todo!(),
                Event::Resize(_columns, _rows) => todo!(),

                Event::FocusGained | Event::FocusLost | Event::Paste(_) => todo!(),
            };

            match event_outcome {
                Some(EventOutcome::Handled) => document.render(&mut buffer),
                Some(EventOutcome::Unhandled) => todo!(),
                None => todo!(),
            }

            self.draw(&buffer, document.visual_cursor_position())?;
        }

        Ok(())
    }

    fn draw(&mut self, buffer: &Buffer, cursor_position: Position) -> io::Result<()> {
        crossterm::queue!(
            self.out,
            crossterm::cursor::Hide,
            crossterm::cursor::MoveTo(0, 0)
        )?;

        let cells = buffer.cells();
        let mut buffer_index = 0;

        while buffer_index < cells.len() {
            let cell = &cells[buffer_index];

            crossterm::queue!(
                self.out,
                style::SetForegroundColor(*cell.foreground()),
                style::SetBackgroundColor(*cell.background()),
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

        self.out.flush()?;

        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Dimensions {
    width: Columns,
    height: Rows,
}

impl Dimensions {
    pub(crate) const fn new(columns: Columns, rows: Rows) -> Self {
        Self {
            width: columns,
            height: rows,
        }
    }

    pub(crate) const fn width(&self) -> &Columns {
        &self.width
    }

    pub(crate) const fn height(&self) -> &Rows {
        &self.height
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    derive_more::From,
    derive_more::Add,
    derive_more::AddAssign,
)]
#[from(forward)]
pub(crate) struct Columns(usize);

impl ops::Add<usize> for Columns {
    type Output = Self;

    fn add(self, rhs: usize) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl ops::AddAssign<usize> for Columns {
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs;
    }
}

impl Columns {
    pub(crate) const fn new(value: usize) -> Self {
        Self(value)
    }

    pub(crate) const fn value(self) -> usize {
        self.0
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    derive_more::From,
    derive_more::Add,
    derive_more::AddAssign,
)]
#[from(forward)]
pub(crate) struct Rows(usize);

impl Rows {
    pub(crate) const fn new(value: usize) -> Self {
        Self(value)
    }

    pub(crate) const fn value(self) -> usize {
        self.0
    }
}

#[derive(Debug)]
#[must_use]
pub enum EventOutcome {
    Handled,
    Unhandled,
}
