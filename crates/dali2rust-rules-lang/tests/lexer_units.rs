use dali2rust_rules_lang::lexer::{lex, Token, TokenKind};

#[test]
fn token_stays_small() {
    let size = std::mem::size_of::<Token>();
    assert!(size <= 40, "Token grew to {size} bytes");
}

fn one(source: &str) -> TokenKind {
    let lexed = lex(source).expect("lexes");
    assert_eq!(lexed.tokens.len(), 1, "{source}");
    lexed.tokens.into_iter().next().map(|t| t.kind).expect("one token")
}

#[test]
fn duration_forms_land_in_milliseconds() {
    assert_eq!(one("500ms"), TokenKind::Duration(500));
    assert_eq!(one("0.7s"), TokenKind::Duration(700));
    assert_eq!(one("5m"), TokenKind::Duration(300_000));
    assert_eq!(one("2h"), TokenKind::Duration(7_200_000));
    assert_eq!(one("1.5m"), TokenKind::Duration(90_000));
}

#[test]
fn sub_millisecond_durations_are_rejected_not_rounded() {
    assert!(lex("0.0001s").is_err());
    assert!(lex("0.5ms").is_err());
}

#[test]
fn times_and_decimals_lex_distinctly() {
    assert_eq!(one("07:30"), TokenKind::Time { hour: 7, minute: 30 });
    assert_eq!(one("0.313"), TokenKind::Decimal1e4(3130));
    assert!(lex("24:00").is_err());
    assert!(lex("07:5").is_err());
}

#[test]
fn a_range_dot_dot_is_not_a_fraction() {
    let lexed = lex("12..15").expect("lexes");
    let kinds: Vec<TokenKind> = lexed.tokens.into_iter().map(|t| t.kind).collect();
    assert_eq!(kinds, [TokenKind::Int(12), TokenKind::DotDot, TokenKind::Int(15)]);
}

#[test]
fn columns_count_characters_not_bytes() {
    let lexed = lex("# кириллица до конца строки\nrule").expect("lexes");
    assert_eq!(lexed.tokens[0].pos.line, 2);
    assert_eq!(lexed.tokens[0].pos.column, 1);
    let lexed = lex("\"имя\" x").expect("lexes");
    assert_eq!(lexed.tokens[1].pos.column, 7);
}
