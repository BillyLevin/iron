use std::{
    fmt::Write as _,
    io::Write as _,
};

use hegel::{
    Generator as _,
    PrintableGenerator,
    TestCase,
    generators,
};

use super::*;

fn doc(test_case: &TestCase, contents: &str) -> Document {
    let mut temp_file = tempfile::NamedTempFile::new().unwrap();
    write!(temp_file, "{contents}").unwrap();

    let width = test_case.draw(generators::integers().min_value(1).map(Columns::new));
    let height = test_case.draw(generators::integers().min_value(1).map(Rows::new));

    Document::new(
        temp_file.path().to_path_buf(),
        Dimensions::new(width, height),
    )
    .unwrap()
}

#[derive(Debug, PartialEq, Eq)]
enum Boundary {
    WordPart,
    Whitespace,
    Other,
}

impl From<char> for Boundary {
    fn from(ch: char) -> Self {
        if ch.is_whitespace() {
            Self::Whitespace
        } else if ch.is_alphanumeric() || ch == '_' {
            Self::WordPart
        } else {
            Self::Other
        }
    }
}

fn delete_word_model(text: &str, cursor: usize) -> (String, usize) {
    let mut chars = text
        .get(cursor..)
        .expect("should be on valid boundary")
        .chars();
    let mut prev_ch = chars.next();
    let mut delete_end = cursor + prev_ch.map_or_default(char::len_utf8);

    for ch in chars {
        if let Some(prev) = prev_ch {
            let prev_boundary = Boundary::from(prev);
            let current_boundary = Boundary::from(ch);

            if prev_boundary != current_boundary && current_boundary != Boundary::Whitespace {
                break;
            }
        }

        prev_ch = Some(ch);
        delete_end += ch.len_utf8();
    }

    let mut result = text.to_owned();
    result.replace_range(cursor..delete_end, "");

    let cursor = cmp::min(
        cursor,
        result
            .char_indices()
            .last()
            .map_or_default(|(index, _)| index),
    );

    (result, cursor)
}

fn move_cursor_next_word_model(text: &str, cursor: usize) -> (String, usize) {
    let mut chars = text
        .get(cursor..)
        .expect("should be on valid boundary")
        .chars();
    let mut prev_ch = chars.next();
    let mut cursor = cursor + prev_ch.map_or_default(char::len_utf8);

    for ch in chars {
        if let Some(prev) = prev_ch {
            let prev_boundary = Boundary::from(prev);
            let current_boundary = Boundary::from(ch);

            if prev_boundary != current_boundary && current_boundary != Boundary::Whitespace {
                break;
            }
        }

        prev_ch = Some(ch);
        cursor += ch.len_utf8();
    }

    (
        text.to_owned(),
        cmp::min(
            cursor,
            text.char_indices()
                .last()
                .map_or_default(|(index, _ch)| index),
        ),
    )
}

fn insert_text_model(text: &str, to_insert: &str, cursor: usize) -> (String, usize) {
    let mut result = text.to_owned();

    result.insert_str(cursor, to_insert);

    let cursor = cmp::min(
        cursor + to_insert.len(),
        result
            .char_indices()
            .last()
            .map_or_default(|(index, _ch)| index),
    );

    (result, cursor)
}

fn move_cursor_first_non_blank_model(text: &str, cursor: usize) -> (String, usize) {
    let mut line_start = 0;

    for (byte_index, ch) in text.char_indices() {
        if ch != '\r' && ch != '\n' {
            continue;
        }

        if ch == '\r' && text.get(byte_index..).unwrap().starts_with("\r\n") {
            continue;
        }

        // we are at '\n'
        let next_line_start = byte_index + 1;

        if next_line_start > cursor {
            break;
        }

        line_start = next_line_start;
    }

    let mut new_cursor = line_start;

    for (offset, ch) in text.get(line_start..).unwrap().char_indices() {
        if ch == '\r' || ch == '\n' {
            break;
        }

        if !ch.is_whitespace() {
            new_cursor = line_start + offset;
            break;
        }
    }

    (text.to_owned(), new_cursor)
}

fn generate_text_and_cursor(test_case: &TestCase) -> (String, usize) {
    let is_empty = test_case.draw(generators::weighted_booleans(0.1));

    if is_empty {
        (String::new(), 0)
    } else {
        let lines = test_case.draw(generators::integers::<u8>().min_value(1).max_value(5));
        let line_index_with_cursor = test_case.draw(
            generators::integers::<u8>()
                .min_value(0)
                .max_value(lines - 1),
        );

        let mut cursor = 0;
        let mut text = String::new();

        for i in 0..lines {
            let is_last = i == lines - 1;

            let prefix = test_case.draw(generators::text().exclude_characters("\r\n"));
            let suffix_start = test_case.draw(hegel::one_of![
                generators::sampled_from(&[' ', '\t', '\u{00A0}', '\u{2003}']),
                generators::characters().filter(|&ch| ch.is_alphanumeric() || ch == '_'),
                generators::characters()
                    .filter(|&ch| !ch.is_whitespace() && !ch.is_alphanumeric() && ch != '_'),
            ]);
            let suffix = test_case.draw(generators::text().exclude_characters("\r\n"));

            let maybe_new_cursor = text.len() + prefix.len();

            let newline = test_case.draw(if is_last {
                generators::sampled_from(&["", "\r\n", "\n"])
            } else {
                generators::sampled_from(&["\r\n", "\n"])
            });

            let _ = write!(text, "{prefix}{suffix_start}{suffix}{newline}");

            if i == line_index_with_cursor {
                let put_cursor_eol = test_case.draw(generators::weighted_booleans(0.1));
                if put_cursor_eol && !newline.is_empty() {
                    cursor = text.len() - newline.len();
                } else {
                    cursor = maybe_new_cursor;
                }
            }
        }

        (text, cursor)
    }
}

#[test]
fn delete_word_model_works() {
    let tests = [
        [("  cat".to_owned(), 0), ("cat".to_owned(), 0)],
        [("é cat".to_owned(), 0), ("cat".to_owned(), 0)],
        [(String::new(), 0), (String::new(), 0)],
        [("hello  world".to_owned(), 0), ("world".to_owned(), 0)],
        [("hello world".to_owned(), 3), ("helworld".to_owned(), 3)],
        [("hello\nworld".to_owned(), 0), ("world".to_owned(), 0)],
        [("hello".to_owned(), 0), (String::new(), 0)],
        [("hello-world".to_owned(), 0), ("-world".to_owned(), 0)],
        [("--hello".to_owned(), 0), ("hello".to_owned(), 0)],
        [("--  hello".to_owned(), 0), ("hello".to_owned(), 0)],
        [("hello_world".to_owned(), 0), (String::new(), 0)],
        [("é_猫 next".to_owned(), 0), ("next".to_owned(), 0)],
        [("é 猫 next".to_owned(), 3), ("é next".to_owned(), 3)],
        [(" \t\ncat".to_owned(), 0), ("cat".to_owned(), 0)],
        [(" \t\n".to_owned(), 0), (String::new(), 0)],
        [("hello  ".to_owned(), 0), (String::new(), 0)],
        [("abc def".to_owned(), 4), ("abc ".to_owned(), 3)],
        [("hello".to_owned(), 3), ("hel".to_owned(), 2)],
        [("é猫".to_owned(), 2), ("é".to_owned(), 0)],
    ];

    for [input, expected] in tests {
        assert_eq!(delete_word_model(&input.0, input.1), expected);
    }
}

#[test]
fn move_cursor_next_word_model_works() {
    let tests = [
        ("", 0, 0),
        (" ", 0, 0),
        ("a", 0, 0),
        ("hello", 0, 4),
        ("hello", 2, 4),
        ("hello", 4, 4),
        ("hello world", 0, 6),
        ("hello world", 3, 6),
        ("hello  world", 5, 7),
        ("  hello", 0, 2),
        ("hello\nworld", 0, 6),
        (" \t\ncat", 0, 3),
        ("hello  ", 0, 6),
        (" \t\n", 0, 2),
        ("hello-world", 0, 5),
        ("hello-world", 5, 6),
        ("--hello", 0, 2),
        ("--  hello", 0, 4),
        ("hello_world next", 0, 12),
        ("é_猫 next", 0, 7),
        ("é猫", 0, 2),
        ("猫", 0, 0),
        ("é 猫", 2, 3),
    ];

    for (text, input, expected) in tests {
        let result = move_cursor_next_word_model(text, input).1;
        assert_eq!(
            result, expected,
            "text: {text}, expected: {expected}, got: {result}"
        );
    }
}

#[hegel::test(test_cases = 1000)]
fn delete_word(test_case: TestCase) {
    let (text, cursor) = generate_text_and_cursor(&test_case);

    if text.is_empty() {
        test_case.event("document: empty");
    } else {
        test_case.event("document: non-empty");

        let first_char = text
            .get(cursor..)
            .expect("should be on valid boundary")
            .chars()
            .next()
            .unwrap();

        let first_char_category = if first_char.is_whitespace() {
            "whitespace"
        } else if first_char.is_alphanumeric() || first_char == '_' {
            "word-part"
        } else {
            "other"
        };

        test_case.event(format!("start: {first_char_category}"));
    }

    let (model_text, model_cursor) = delete_word_model(&text, cursor);

    let deleted_len = text.len() - model_text.len();
    let delete_end = cursor + deleted_len;
    let deleted = text
        .get(cursor..delete_end)
        .expect("should be on valid boundary");

    if !text.is_empty() {
        if delete_end == text.len() {
            test_case.event("stop: EOF");
        } else {
            let stop_char = text
                .get(delete_end..)
                .expect("should be on valid boundary")
                .chars()
                .next()
                .unwrap();

            let stop_char_category = if stop_char.is_whitespace() {
                "whitespace"
            } else if stop_char.is_alphanumeric() || stop_char == '_' {
                "word-part"
            } else {
                "other"
            };

            test_case.event(format!("stop: {stop_char_category}"));
        }
    }

    if deleted.chars().any(|ch| ch.len_utf8() > 1) {
        test_case.event("deleted: contains multi-byte");
    }

    let mut document = doc(&test_case, &text);
    document.set_cursor(ByteIndex::new(cursor));

    for key in ['d', 'w'] {
        let _ =
            document.handle_key_event(KeyEvent::from(KeyCode::Char(key)), &mut EventContext::new());
    }

    assert_eq!(
        (document.text.to_string(), document.cursor().value()),
        (model_text, model_cursor)
    );
}

#[hegel::test(test_cases = 1000)]
fn delete_word_sequence(test_case: TestCase) {
    let (mut text, mut cursor) = generate_text_and_cursor(&test_case);

    let command_count = test_case.draw(generators::integers::<u8>().min_value(1).max_value(5));

    test_case.event(format!("sequence length: {command_count}"));

    let mut document = doc(&test_case, &text);
    document.set_cursor(ByteIndex::new(cursor));

    for i in 0..command_count {
        if text.is_empty() {
            test_case.event("command: executed on empty document");
        }

        (text, cursor) = delete_word_model(&text, cursor);

        for key in ['d', 'w'] {
            let _ = document
                .handle_key_event(KeyEvent::from(KeyCode::Char(key)), &mut EventContext::new());
        }

        assert_eq!(
            (&document.text.to_string(), document.cursor().value()),
            (&text, cursor),
            "step index: {i}"
        );
    }
}

fn insertion_text_generator() -> impl PrintableGenerator<String> {
    generators::text()
        // letters, numbers, punctuation, symbols
        .categories(&["L", "N", "P", "S"])
        .include_characters(" \n\t")
        .max_size(40)
}

#[hegel::test(test_cases = 1000)]
fn command_sequence(test_case: TestCase) {
    #[derive(Debug, Clone, hegel::DefaultGenerator)]
    enum Command {
        DeleteWord,
        MoveCursorNextWord,
        MoveCursorFirstNonBlank,
        Insert(String),
    }

    let (mut text, mut cursor) = generate_text_and_cursor(&test_case);

    test_case.event(format!("lines: {}", cmp::max(text.lines().count(), 1)));

    let commands = test_case.draw(
        generators::vecs(generators::default::<Command>().insert(insertion_text_generator()))
            .min_size(1)
            .max_size(20),
    );

    test_case.event(format!("sequence length: {}", commands.len()));

    let mut document = doc(&test_case, &text);
    document.set_cursor(ByteIndex::new(cursor));

    for (i, command) in commands.into_iter().enumerate() {
        if text.is_empty() {
            test_case.event("command: executed on empty document");
        }

        let keys;
        ((text, cursor), keys) = match command {
            Command::DeleteWord => {
                (delete_word_model(&text, cursor), vec![
                    KeyCode::Char('d'),
                    KeyCode::Char('w'),
                ])
            }
            Command::MoveCursorNextWord => {
                (move_cursor_next_word_model(&text, cursor), vec![
                    KeyCode::Char('w'),
                ])
            }
            Command::Insert(content) => {
                let mut insertion_keys = vec![KeyCode::Char('i')];

                for ch in content.chars() {
                    match ch {
                        '\n' => insertion_keys.push(KeyCode::Enter),
                        '\t' => insertion_keys.push(KeyCode::Tab),
                        _ => insertion_keys.push(KeyCode::Char(ch)),
                    }
                }

                insertion_keys.push(KeyCode::Esc);

                (insert_text_model(&text, &content, cursor), insertion_keys)
            }
            Command::MoveCursorFirstNonBlank => {
                (move_cursor_first_non_blank_model(&text, cursor), vec![
                    KeyCode::Char('^'),
                ])
            }
        };

        for key in keys {
            let _ = document.handle_key_event(KeyEvent::from(key), &mut EventContext::new());
        }

        assert_eq!(
            (&document.text.to_string(), document.cursor().value()),
            (&text, cursor),
            "step index: {i}"
        );
    }
}
