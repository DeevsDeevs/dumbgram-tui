use base64::Engine;
use std::io::Write;

const OSC52_PREFIX: &str = "\x1b]52;c;";
const OSC52_SUFFIX: &str = "\x07";

pub(crate) fn osc52_copy_sequence(text: &str) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    format!("{OSC52_PREFIX}{encoded}{OSC52_SUFFIX}")
}

pub(crate) fn copy_text<W: Write>(writer: &mut W, text: &str) -> std::io::Result<()> {
    writer.write_all(osc52_copy_sequence(text).as_bytes())?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::{copy_text, osc52_copy_sequence};

    #[test]
    fn osc52_copy_sequence_encodes_text_without_plaintext() {
        let sequence = osc52_copy_sequence("secret message");

        assert!(sequence.starts_with("\x1b]52;c;"));
        assert!(sequence.ends_with('\x07'));
        assert!(sequence.contains("c2VjcmV0IG1lc3NhZ2U="));
        assert!(!sequence.contains("secret message"));
    }

    #[test]
    fn copy_text_writes_osc52_sequence() {
        let mut output = Vec::new();

        copy_text(&mut output, "hello").unwrap();

        assert_eq!(String::from_utf8(output).unwrap(), "\x1b]52;c;aGVsbG8=\x07");
    }
}
