use std::{
    cmp,
    iter,
};

use crossterm::event::{
    KeyCode,
    KeyEvent,
    KeyModifiers,
};
use hegel::{
    Generator as _,
    PrettyPrinter,
    TestCase,
    generators,
};
use itertools::Itertools as _;

use crate::{
    document::Mode,
    keymap::{
        KeyBinding,
        KeyMap,
    },
    test_harness::TestHarness,
    ui::{
        Columns,
        EditorDimensions,
        Rectangle,
        Rows,
    },
};

struct Fuzzer {
    harness: TestHarness,
    bindings: Vec<(Mode, Vec<KeyEvent>)>,
}

#[hegel::state_machine]
#[expect(
    clippy::needless_pass_by_value,
    reason = "hegel requires us to take `TestCase` by value"
)]
impl Fuzzer {
    fn new(test_case: &TestCase) -> Self {
        let harness = TestHarness::new(&draw_contents(test_case), draw_dimensions(test_case));

        let bindings = [Mode::Normal, Mode::Visual, Mode::Insert]
            .into_iter()
            .flat_map(|mode| {
                KeyMap::for_mode(mode)
                    .bindings()
                    .into_iter()
                    .map(move |keys| {
                        (
                            mode,
                            keys.into_iter()
                                .map(|key| KeyEvent::new(key.code(), key.modifiers()))
                                .collect(),
                        )
                    })
            })
            .collect();

        Self { harness, bindings }
    }

    #[rule]
    fn send_keybinding(&mut self, test_case: TestCase) {
        test_case.assume(!self.harness.is_closed());

        let match_current_mode = test_case.draw_silent(generators::weighted_booleans(0.9));
        let mode_filter = match_current_mode.then(|| self.harness.editor().document().mode());

        let keys = test_case.draw(
            generators::sampled_from(
                self.bindings
                    .iter()
                    .filter(|&&(binding_mode, _)| {
                        mode_filter.is_none_or(|mode| mode == binding_mode)
                    })
                    .map(|&(_, ref keys)| keys.clone())
                    .collect::<Vec<_>>(),
            )
            .print_with(|keys, printer| print_keys(keys, printer)),
        );

        for key in keys {
            self.harness.send_key(key);
        }
    }

    #[rule]
    fn send_key(&mut self, test_case: TestCase) {
        test_case.assume(!self.harness.is_closed());

        let code = test_case.draw_silent(hegel::one_of![
            generators::characters().map(KeyCode::Char),
            generators::sampled_from(vec![
                KeyCode::Esc,
                KeyCode::Enter,
                KeyCode::Backspace,
                KeyCode::Delete,
                KeyCode::Tab,
                KeyCode::Left,
                KeyCode::Right,
                KeyCode::Up,
                KeyCode::Down,
            ]),
        ]);

        let use_modifier = test_case.draw_silent(generators::weighted_booleans(0.05));

        let modifiers = if use_modifier {
            test_case.draw_silent(generators::sampled_from(vec![
                KeyModifiers::SHIFT,
                KeyModifiers::CONTROL,
                KeyModifiers::ALT,
            ]))
        } else {
            KeyModifiers::NONE
        };

        let key = KeyEvent::new(code, modifiers);
        test_case.note(&format!("key: {}", KeyBinding::from(key)));

        self.harness.send_key(key);
    }

    #[rule]
    fn resize(&mut self, test_case: TestCase) {
        test_case.assume(!self.harness.is_closed());

        let dimensions = draw_dimensions(&test_case);
        self.harness.resize(dimensions);
    }

    #[invariant(always_run)]
    fn cursor_always_on_screen(&self, test_case: TestCase) {
        let screen = self.harness.screen();
        let cursor = screen.cursor();
        let dimensions = screen.dimensions();

        test_case.note(&screen.to_string());

        assert!(
            Rectangle::from_dimensions(dimensions).contains(&cursor),
            "cursor at {cursor:?} is outside the screen ({dimensions:?})"
        );
    }
}

fn print_keys(keys: &[KeyEvent], printer: &mut PrettyPrinter) {
    let keys = keys
        .iter()
        .map(|&key| KeyBinding::from(key).to_string())
        .collect::<Vec<_>>()
        .join(" ");

    printer.text(&format!("{keys:?}"));
}

#[derive(Debug, Clone, Copy, hegel::DefaultGenerator)]
enum Indent {
    Tab,
    TwoSpaces,
    FourSpaces,
}

impl Indent {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tab => "\t",
            Self::TwoSpaces => "  ",
            Self::FourSpaces => "    ",
        }
    }
}

/// Draws document contents. It mostly does regular text generation with a
/// couple of changes:
/// - repeated lines so that we can test large files without having to generate
/// - deliberate indent/starting whitespace on the lines as this is common in
///   code but not common in regular text generation
fn draw_contents(test_case: &TestCase) -> String {
    let line_count_ranges = [
        (5, "0", 0..=0),
        (15, "1-9", 1..=9),
        (20, "10-99", 10..=99),
        (35, "100-999", 100..=999),
        (20, "1,000-9,999", 1_000..=9_999),
        (5, "10,000+", 10_000..=20_000),
    ]
    .into_iter()
    .flat_map(|(weight, label, line_counts)| iter::repeat_n((label, line_counts), weight))
    .collect::<Vec<_>>();

    let (label, line_counts) = test_case.draw_silent(generators::sampled_from(line_count_ranges));
    test_case.event(format!("lines: {label}"));

    let line_count = test_case.draw(
        generators::integers::<usize>()
            .min_value(*line_counts.start())
            .max_value(*line_counts.end()),
    );

    if line_count == 0 {
        return String::new();
    }

    let indent = test_case.draw_silent(generators::default::<Indent>());
    test_case.event(format!("indent: {indent:?}"));

    let trailing_newline = test_case.draw_silent(generators::booleans());
    test_case.event(format!("trailing newline: {trailing_newline}"));

    let mut contents = test_case
        .draw(
            generators::vecs(
                hegel::tuples!(
                    generators::integers::<usize>().max_value(3),
                    generators::text().max_size(80).exclude_characters("\n\r"),
                )
                .map(move |(indent_repeats, text)| {
                    format!("{}{text}", indent.as_str().repeat(indent_repeats))
                }),
            )
            .min_size(1)
            .max_size(cmp::min(line_count, 100)),
        )
        .iter()
        .cycle()
        .take(line_count)
        .join("\n");

    if trailing_newline {
        contents.push('\n');
    }

    contents
}

fn draw_dimensions(test_case: &TestCase) -> EditorDimensions {
    EditorDimensions::draw(test_case, Columns::new(100), Rows::new(40))
}

#[hegel::test]
fn fuzz_editor(test_case: TestCase) {
    hegel::stateful::machine(Fuzzer::new(&test_case)).run(test_case);
}
