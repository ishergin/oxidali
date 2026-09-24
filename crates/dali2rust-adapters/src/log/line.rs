#![allow(
    dead_code,
    reason = "the ESP-only caller is cfg'd out on host; the tests below are the point"
)]

const ANSI_RESET: &[u8] = b"\x1b[0m";

pub(crate) fn trim_tail(rendered: &[u8]) -> &[u8] {
    strip_suffix(strip_suffix(rendered, b"\n"), ANSI_RESET)
}

fn strip_suffix<'a>(bytes: &'a [u8], suffix: &[u8]) -> &'a [u8] {
    match bytes.strip_suffix(suffix) {
        Some(trimmed) => trimmed,
        None => bytes,
    }
}

pub(crate) fn split_line(rendered: &[u8]) -> (&[u8], &[u8]) {
    let after_stamp = rendered
        .windows(2)
        .position(|pair| pair == b") ")
        .map_or(0, |index| index + 2);
    let rest = &rendered[after_stamp..];
    match rest.windows(2).position(|pair| pair == b": ") {
        Some(split) => (&rest[..split], &rest[split + 2..]),
        None => (&[], rest),
    }
}

#[cfg(test)]
mod tests {
    use super::{split_line, trim_tail};

    #[test]
    fn a_message_ending_in_a_digit_keeps_every_digit() {
        assert_eq!(trim_tail(b"count 100\n"), b"count 100");
        assert_eq!(trim_tail(b"Heap free: 240\n"), b"Heap free: 240");
        assert_eq!(trim_tail(b"hwm 1460\n"), b"hwm 1460");
        assert_eq!(trim_tail(b"level 0\n"), b"level 0");
    }

    #[test]
    fn a_message_ending_in_a_reset_byte_keeps_it() {
        assert_eq!(trim_tail(b"gpio[0]\n"), b"gpio[0]");
        assert_eq!(trim_tail(b"trailing;\n"), b"trailing;");
        assert_eq!(trim_tail(b"in dbm\n"), b"in dbm");
    }

    #[test]
    fn the_ansi_reset_is_still_stripped_when_colours_are_on() {
        assert_eq!(trim_tail(b"I (12) x: val 100\x1b[0m\n"), b"I (12) x: val 100");
        assert_eq!(trim_tail(b"I (12) x: ok\x1b[0m\n"), b"I (12) x: ok");
    }

    #[test]
    fn a_line_with_nothing_to_trim_is_returned_whole() {
        assert_eq!(trim_tail(b"no newline"), b"no newline");
        assert_eq!(trim_tail(b""), b"");
    }

    #[test]
    fn a_macro_line_splits_into_tag_and_message() {
        let (tag, text) = split_line(b"I (12345) httpd: recv error=100");
        assert_eq!(tag, b"httpd");
        assert_eq!(text, b"recv error=100");
    }

    #[test]
    fn a_line_without_the_macro_shape_is_all_message() {
        let (tag, text) = split_line(b"bare component output");
        assert_eq!(tag, b"");
        assert_eq!(text, b"bare component output");
    }
}
