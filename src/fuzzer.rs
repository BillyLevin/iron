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
        Dimensions,
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
}

#[hegel::test]
fn fuzz_editor(test_case: TestCase) {
    let harness = TestHarness::new(
        "hello\nthere\nworld",
        Dimensions::new(Columns::new(40), Rows::new(10)),
    );

    hegel::stateful::machine(Fuzzer { harness }).run(test_case);
}
