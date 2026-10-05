pub const fn flag_knob(raw: Option<&str>) -> Option<bool> {
    match raw {
        None => None,
        Some(value) => match value.as_bytes() {
            b"" => None,
            b"1" => Some(true),
            b"0" => Some(false),
            _ => panic!("a DALI2RUST_* flag knob is 1, 0 or unset"),
        },
    }
}

pub const fn flag_knob_on(raw: Option<&str>) -> bool {
    matches!(flag_knob(raw), Some(true))
}

const _: () = assert!(matches!(flag_knob(Some("1")), Some(true)));
const _: () = assert!(matches!(flag_knob(Some("0")), Some(false)));
const _: () = assert!(flag_knob(Some("")).is_none() && flag_knob(None).is_none());
const _: () = assert!(flag_knob_on(Some("1")) && !flag_knob_on(Some("0")) && !flag_knob_on(None));
