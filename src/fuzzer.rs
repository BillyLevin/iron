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
        let harness = TestHarness::new("hello\nthere\nworld", draw_dimensions(test_case));

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

fn draw_dimensions(test_case: &TestCase) -> EditorDimensions {
    EditorDimensions::draw(test_case, Columns::new(100), Rows::new(40))
}

#[hegel::test]
fn fuzz_editor(test_case: TestCase) {
    hegel::stateful::machine(Fuzzer::new(&test_case)).run(test_case);
}
