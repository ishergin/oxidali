pub const MAX_JSON_DEPTH: usize = 8;

pub fn json_too_deep(body: &[u8]) -> bool {
    depth_exceeds(body, MAX_JSON_DEPTH)
}

fn depth_exceeds(body: &[u8], max_depth: usize) -> bool {
    let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
    for &b in body {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > max_depth {
                    return true;
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

#[cfg(test)]
pub(crate) fn nested_json(depth: usize) -> Vec<u8> {
    let mut s = String::from("{\"a\":");
    s.push_str(&"[".repeat(depth - 1));
    s.push('1');
    s.push_str(&"]".repeat(depth - 1));
    s.push('}');
    s.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_depths_pass() {
        assert!(!json_too_deep(&nested_json(MAX_JSON_DEPTH)));
        assert!(serde_json::from_slice::<serde_json::Value>(&nested_json(MAX_JSON_DEPTH)).is_ok());
    }

    #[test]
    fn one_level_past_the_limit_is_too_deep() {
        assert!(json_too_deep(&nested_json(MAX_JSON_DEPTH + 1)));
    }

    #[test]
    fn brackets_inside_strings_do_not_count() {
        assert!(!json_too_deep(br#"{"a":"[[[[[[[[[[[[[[[[[[[["}"#));
    }

    #[test]
    fn escaped_quotes_keep_the_string_open() {
        assert!(!json_too_deep(br#"{"a":"x\"[[[[[[[[[[\"y"}"#));
    }
}
