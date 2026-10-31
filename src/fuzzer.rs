use crossterm::event::{
    KeyCode,
    KeyEvent,
    KeyModifiers,
};
use hegel::{
    Generator as _,
    TestCase,
    generators,
};

use crate::{
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
}

#[hegel::state_machine]
#[expect(
    clippy::needless_pass_by_value,
    reason = "hegel requires us to take `TestCase` by value"
)]
impl Fuzzer {
    #[rule]
    fn send_key(&mut self, test_case: TestCase) {
        test_case.assume(!self.harness.is_closed());

        let code = test_case.draw(
            hegel::one_of![
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
            ]
            .print_as_debug(),
        );

        let modifiers = test_case.draw(
            generators::sampled_from(vec![
                KeyModifiers::NONE,
                KeyModifiers::SHIFT,
                KeyModifiers::CONTROL,
                KeyModifiers::ALT,
            ])
            .print_as_debug(),
        );

        self.harness.send_key(KeyEvent::new(code, modifiers));
    }

    #[rule]
    fn resize(&mut self, test_case: TestCase) {
        test_case.assume(!self.harness.is_closed());

        let dimensions = draw_dimensions(&test_case);
        self.harness.resize(dimensions);
    }

    #[invariant(always_run)]
    fn cursor_always_on_screen(&self, _test_case: TestCase) {
        let screen = self.harness.screen();
        let cursor = screen.cursor();
        let dimensions = screen.dimensions();

        assert!(
            Rectangle::from_dimensions(dimensions).contains(&cursor),
            "cursor at {cursor:?} is outside the screen ({dimensions:?})"
        );
    }
}

fn draw_dimensions(test_case: &TestCase) -> EditorDimensions {
    EditorDimensions::draw(test_case, Columns::new(100), Rows::new(40))
}

#[hegel::test]
fn fuzz_editor(test_case: TestCase) {
    let harness = TestHarness::new("hello\nthere\nworld", draw_dimensions(&test_case));

    hegel::stateful::machine(Fuzzer { harness }).run(test_case);
}
