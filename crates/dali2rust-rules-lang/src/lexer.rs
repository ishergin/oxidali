use dali2rust_rules_model::CompileError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    pub line: u32,
    pub column: u32,
}

impl Pos {
    pub fn err(self, message: impl Into<String>) -> CompileError {
        CompileError::at(self.line, self.column, message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Ident(String),
    Str(String),
    Int(i64),
    Time { hour: u8, minute: u8 },
    Duration(u32),
    Decimal1e4(u32),
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Dot,
    DotDot,
    Assign,
    EqEq,
    Ne,
    Ge,
    Le,
    Gt,
    Lt,
    Plus,
    Minus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub pos: Pos,
}

#[derive(Debug)]
pub struct Lexed {
    pub tokens: Vec<Token>,
    pub end: Pos,
}

struct Lexer<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    line: u32,
    column: u32,
}

const MS_PER_SECOND: u64 = 1000;
const MS_PER_MINUTE: u64 = 60_000;
const MS_PER_HOUR: u64 = 3_600_000;
const DECIMAL_PLACES: u32 = 4;
const MAX_INT_LITERAL: i64 = 1_000_000_000;

pub fn lex(source: &str) -> Result<Lexed, CompileError> {
    let mut lx = Lexer {
        chars: source.chars().peekable(),
        line: 1,
        column: 1,
    };
    let mut tokens = Vec::new();
    loop {
        lx.skip_trivia();
        let pos = lx.pos();
        let Some(&c) = lx.chars.peek() else {
            return Ok(Lexed { tokens, end: pos });
        };
        let kind = lx.token_at(c, pos)?;
        tokens.push(Token { kind, pos });
    }
}

impl<'a> Lexer<'a> {
    fn pos(&self) -> Pos {
        Pos {
            line: self.line,
            column: self.column,
        }
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.chars.next()?;
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(c)
    }

    fn skip_trivia(&mut self) {
        while let Some(&c) = self.chars.peek() {
            if c.is_whitespace() {
                self.bump();
            } else if c == '#' {
                while let Some(&c) = self.chars.peek() {
                    if c == '\n' {
                        break;
                    }
                    self.bump();
                }
            } else {
                break;
            }
        }
    }

    fn token_at(&mut self, c: char, pos: Pos) -> Result<TokenKind, CompileError> {
        if c == '"' {
            return self.string(pos);
        }
        if c.is_ascii_digit() {
            return self.number(pos);
        }
        if c.is_ascii_alphabetic() || c == '_' {
            return Ok(self.ident());
        }
        self.punct(c, pos)
    }

    fn string(&mut self, pos: Pos) -> Result<TokenKind, CompileError> {
        self.bump();
        let mut s = String::new();
        loop {
            match self.chars.peek() {
                None => return Err(pos.err("unterminated string")),
                Some('\n') => return Err(pos.err("unterminated string")),
                Some('"') => {
                    self.bump();
                    return Ok(TokenKind::Str(s));
                }
                Some(&c) => {
                    s.push(c);
                    self.bump();
                }
            }
        }
    }

    fn ident(&mut self) -> TokenKind {
        let mut s = String::new();
        while let Some(&c) = self.chars.peek() {
            if c.is_ascii_alphanumeric() || c == '_' {
                s.push(c);
                self.bump();
            } else {
                break;
            }
        }
        TokenKind::Ident(s)
    }

    fn digits(&mut self, pos: Pos) -> Result<(i64, u32), CompileError> {
        let mut value: i64 = 0;
        let mut count: u32 = 0;
        while let Some(&c) = self.chars.peek() {
            let Some(d) = c.to_digit(10) else { break };
            value = value * 10 + i64::from(d);
            count += 1;
            if value > MAX_INT_LITERAL {
                return Err(pos.err("number too large"));
            }
            self.bump();
        }
        Ok((value, count))
    }

    fn number(&mut self, pos: Pos) -> Result<TokenKind, CompileError> {
        let (int, int_digits) = self.digits(pos)?;
        let next = self.chars.peek().copied();
        match next {
            Some(':') => self.time(int, int_digits, pos),
            Some('.') if self.peek_is_frac() => self.fraction(int, pos),
            Some(c) if c.is_ascii_alphabetic() => self.duration(int, 0, 0, pos),
            _ => Ok(TokenKind::Int(int)),
        }
    }

    fn peek_is_frac(&mut self) -> bool {
        let mut probe = self.chars.clone();
        probe.next();
        matches!(probe.peek(), Some(c) if c.is_ascii_digit())
    }

    fn time(&mut self, hour: i64, hour_digits: u32, pos: Pos) -> Result<TokenKind, CompileError> {
        self.bump();
        let (minute, digits) = self.digits(pos)?;
        if hour_digits == 0 || hour_digits > 2 || digits != 2 || hour > 23 || minute > 59 {
            return Err(pos.err("invalid time, expected HH:MM"));
        }
        Ok(TokenKind::Time {
            hour: hour as u8,
            minute: minute as u8,
        })
    }

    fn fraction(&mut self, int: i64, pos: Pos) -> Result<TokenKind, CompileError> {
        self.bump();
        let (frac, frac_digits) = self.digits(pos)?;
        if frac_digits == 0 {
            return Err(pos.err("digits expected after decimal point"));
        }
        match self.chars.peek() {
            Some(c) if c.is_ascii_alphabetic() => self.duration(int, frac, frac_digits, pos),
            _ => self.decimal(int, frac, frac_digits, pos),
        }
    }

    fn decimal(&mut self, int: i64, frac: i64, digits: u32, pos: Pos) -> Result<TokenKind, CompileError> {
        if digits > DECIMAL_PLACES {
            return Err(pos.err("more than 4 decimal places"));
        }
        let scale = 10i64.pow(DECIMAL_PLACES);
        let value = int
            .checked_mul(scale)
            .and_then(|v| v.checked_add(frac * 10i64.pow(DECIMAL_PLACES - digits)));
        match value {
            Some(v) if v <= i64::from(u32::MAX) => Ok(TokenKind::Decimal1e4(v as u32)),
            _ => Err(pos.err("number too large")),
        }
    }

    fn unit_ms(&mut self, pos: Pos) -> Result<u64, CompileError> {
        let mut unit = String::new();
        while let Some(&c) = self.chars.peek() {
            if c.is_ascii_alphabetic() {
                unit.push(c);
                self.bump();
            } else {
                break;
            }
        }
        match unit.as_str() {
            "ms" => Ok(1),
            "s" => Ok(MS_PER_SECOND),
            "m" => Ok(MS_PER_MINUTE),
            "h" => Ok(MS_PER_HOUR),
            _ => Err(pos.err(format!("unknown duration unit \"{unit}\""))),
        }
    }

    fn duration(&mut self, int: i64, frac: i64, frac_digits: u32, pos: Pos) -> Result<TokenKind, CompileError> {
        let unit = self.unit_ms(pos)?;
        let scale = 10u64.pow(frac_digits);
        let total = (int as u64)
            .checked_mul(unit)
            .and_then(|w| w.checked_mul(scale))
            .and_then(|w| w.checked_add(frac as u64 * unit));
        let Some(total) = total else {
            return Err(pos.err("duration too large"));
        };
        if total % scale != 0 {
            return Err(pos.err("duration finer than 1 ms"));
        }
        let ms = total / scale;
        u32::try_from(ms)
            .map(TokenKind::Duration)
            .map_err(|_| pos.err("duration too large"))
    }

    fn punct(&mut self, c: char, pos: Pos) -> Result<TokenKind, CompileError> {
        self.bump();
        let two = |lx: &mut Lexer<'a>, second: char| -> bool {
            if lx.chars.peek() == Some(&second) {
                lx.bump();
                true
            } else {
                false
            }
        };
        match c {
            '(' => Ok(TokenKind::LParen),
            ')' => Ok(TokenKind::RParen),
            '{' => Ok(TokenKind::LBrace),
            '}' => Ok(TokenKind::RBrace),
            ',' => Ok(TokenKind::Comma),
            '+' => Ok(TokenKind::Plus),
            '-' => Ok(TokenKind::Minus),
            '.' => Ok(if two(self, '.') { TokenKind::DotDot } else { TokenKind::Dot }),
            '=' => Ok(if two(self, '=') { TokenKind::EqEq } else { TokenKind::Assign }),
            '>' => Ok(if two(self, '=') { TokenKind::Ge } else { TokenKind::Gt }),
            '<' => Ok(if two(self, '=') { TokenKind::Le } else { TokenKind::Lt }),
            '!' if two(self, '=') => Ok(TokenKind::Ne),
            _ => Err(pos.err(format!("unexpected character '{c}'"))),
        }
    }
}
