use std::io;

use anyhow::Context as _;

fn read_message<Input>(input: &mut Input) -> anyhow::Result<serde_json::Value>
where
    Input: io::BufRead,
{
    let mut buffer = String::new();
    let mut content_length: Option<usize> = None;

    loop {
        buffer.clear();

        anyhow::ensure!(
            input.read_line(&mut buffer)? > 0,
            "reached end of message before it could be parsed"
        );

        if buffer == "\r\n" {
            break;
        }

        match buffer.split_once(": ") {
            Some((key, value)) if key.eq_ignore_ascii_case("Content-Length") => {
                content_length = Some(
                    value
                        .trim()
                        .parse()
                        .context("content length is not a valid `usize`")?,
                );
            }
            Some(_) | None => {}
        }
    }

    let mut content = vec![0_u8; content_length.context("no content length found")?];
    input
        .read_exact(&mut content)
        .context("failed to read content")?;

    serde_json::from_slice(&content).context("failed to convert content to json")
}

fn write_message<Output>(output: &mut Output, message: &serde_json::Value) -> anyhow::Result<()>
where
    Output: io::Write,
{
    let message_bytes = serde_json::to_vec(message)?;

    output
        .write_all(format!("Content-Length: {}\r\n\r\n", message_bytes.len()).as_bytes())
        .context("failed to write header")?;

    output
        .write_all(&message_bytes)
        .context("failed to write body")?;

    output.flush().context("failed to flush")
}

#[cfg(test)]
mod tests {
    use std::io;

    use serde_json::json;

    use super::*;

    fn message(body: &str) -> Vec<u8> {
        format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    }

    fn message_with_headers(headers: &str, body: &str) -> Vec<u8> {
        format!("{headers}\r\n{body}").into_bytes()
    }

    #[test]
    fn read_message_reads_json_body() {
        let mut input = io::Cursor::new(message(r#"{"jsonrpc":"2.0","id":1}"#));

        assert_eq!(
            read_message(&mut input).unwrap(),
            json!({"jsonrpc": "2.0", "id": 1_i32})
        );
    }

    #[test]
    fn read_message_requires_content_length() {
        let mut input = io::Cursor::new(b"\r\n{}".as_slice());
        assert!(read_message(&mut input).is_err());
    }

    #[test]
    fn read_message_ignores_content_type() {
        let body = r#"{"method":"initialized"}"#;
        let mut input = io::Cursor::new(message_with_headers(
            &format!(
                "Content-Length: {}\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n",
                body.len()
            ),
            body,
        ));

        assert_eq!(
            read_message(&mut input).unwrap(),
            json!({"method": "initialized"})
        );
    }

    #[test]
    fn read_message_accepts_case_insensitive_headers() {
        let body = r#"{"method":"initialized"}"#;
        let mut input = io::Cursor::new(message_with_headers(
            &format!("content-length: {}\r\n", body.len()),
            body,
        ));

        assert_eq!(
            read_message(&mut input).unwrap(),
            json!({"method": "initialized"})
        );
    }

    #[test]
    fn read_message_reads_one_message_at_a_time() {
        let first = message(r#"{"id":1}"#);
        let second = message(r#"{"id":2}"#);
        let mut input = io::Cursor::new([first, second].concat());

        assert_eq!(read_message(&mut input).unwrap(), json!({"id": 1_i32}));
        assert_eq!(read_message(&mut input).unwrap(), json!({"id": 2_i32}));
    }

    #[test]
    fn read_message_rejects_invalid_json() {
        let mut input = io::Cursor::new(message("not json"));
        assert!(read_message(&mut input).is_err());
    }

    #[test]
    fn write_message_writes_content_length_and_body() {
        let mut output = Vec::new();

        write_message(&mut output, &json!({"id": 1_i32})).unwrap();

        assert_eq!(output, b"Content-Length: 8\r\n\r\n{\"id\":1}".as_slice());
    }
}
