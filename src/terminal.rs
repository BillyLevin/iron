use std::{
    io::{self, Write as _},
    panic,
};

use crossterm::{
    event::{self, Event, KeyCode},
    style,
};

use crate::{args::Args, buffer::Buffer, document::Document};

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
        let document = Document::new(&args.file_path)?;
        let mut buffer = Buffer::new(self.dimensions);

        document.render(&mut buffer, &self.dimensions);
        self.draw(&buffer)?;

        loop {
            match event::read()? {
                Event::Key(key_event) => {
                    if key_event.code == KeyCode::Char('q') {
                        break;
                    }
                }
                Event::Mouse(_mouse_event) => todo!(),
                Event::Resize(_columns, _rows) => todo!(),

                Event::FocusGained | Event::FocusLost | Event::Paste(_) => {}
            }

            document.render(&mut buffer, &self.dimensions);
            self.draw(&buffer)?;
        }

        Ok(())
    }

    fn draw(&mut self, buffer: &Buffer) -> io::Result<()> {
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

        crossterm::queue!(self.out, crossterm::cursor::Show)?;

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
    fn new(columns: Columns, rows: Rows) -> Self {
        Self {
            width: columns,
            height: rows,
        }
    }

    pub(crate) fn width(&self) -> &Columns {
        &self.width
    }

    pub(crate) fn height(&self) -> &Rows {
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

impl Columns {
    pub(crate) fn value(self) -> usize {
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
    pub(crate) fn value(self) -> usize {
        self.0
    }
}
