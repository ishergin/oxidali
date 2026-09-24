use crate::lexer::{Lexed, Pos, Token, TokenKind};
use dali2rust_rules_model::{CompileError, NameResolver};

pub struct Cursor<'a> {
    tokens: Vec<Token>,
    index: usize,
    end: Pos,
    pub resolver: &'a dyn NameResolver,
}

impl<'a> Cursor<'a> {
    pub fn new(lexed: Lexed, resolver: &'a dyn NameResolver) -> Cursor<'a> {
        Cursor {
            tokens: lexed.tokens,
            index: 0,
            end: lexed.end,
            resolver,
        }
    }

    pub fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.index)
    }

    pub fn peek_second(&self) -> Option<&Token> {
        self.tokens.get(self.index + 1)
    }

    pub fn advance(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.index).cloned();
        if token.is_some() {
            self.index += 1;
        }
        token
    }

    pub fn here(&self) -> Pos {
        self.peek().map_or(self.end, |t| t.pos)
    }

    pub fn err_here(&self, message: impl Into<String>) -> CompileError {
        self.here().err(message)
    }

    pub fn accept(&mut self, kind: &TokenKind) -> bool {
        if self.peek().map(|t| &t.kind) == Some(kind) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    pub fn expect(&mut self, kind: &TokenKind, what: &str) -> Result<Pos, CompileError> {
        let pos = self.here();
        if self.accept(kind) {
            Ok(pos)
        } else {
            Err(pos.err(format!("expected {what}")))
        }
    }

    pub fn accept_kw(&mut self, kw: &str) -> bool {
        match self.peek() {
            Some(Token { kind: TokenKind::Ident(s), .. }) if s == kw => {
                self.index += 1;
                true
            }
            _ => false,
        }
    }

    pub fn peek_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Some(Token { kind: TokenKind::Ident(s), .. }) if s == kw)
    }

    pub fn expect_kw(&mut self, kw: &str) -> Result<Pos, CompileError> {
        let pos = self.here();
        if self.accept_kw(kw) {
            Ok(pos)
        } else {
            Err(pos.err(format!("expected `{kw}`")))
        }
    }

    pub fn expect_ident(&mut self, what: &str) -> Result<(String, Pos), CompileError> {
        let pos = self.here();
        match self.advance() {
            Some(Token { kind: TokenKind::Ident(s), .. }) => Ok((s, pos)),
            _ => Err(pos.err(format!("expected {what}"))),
        }
    }

    pub fn expect_string(&mut self, what: &str) -> Result<(String, Pos), CompileError> {
        let pos = self.here();
        match self.advance() {
            Some(Token { kind: TokenKind::Str(s), .. }) => Ok((s, pos)),
            _ => Err(pos.err(format!("expected quoted {what}"))),
        }
    }

    pub fn expect_int(&mut self, what: &str) -> Result<(i64, Pos), CompileError> {
        let pos = self.here();
        match self.advance() {
            Some(Token { kind: TokenKind::Int(n), .. }) => Ok((n, pos)),
            _ => Err(pos.err(format!("expected {what}"))),
        }
    }

    pub fn expect_duration(&mut self, what: &str) -> Result<(u32, Pos), CompileError> {
        let pos = self.here();
        match self.advance() {
            Some(Token { kind: TokenKind::Duration(ms), .. }) => Ok((ms, pos)),
            _ => Err(pos.err(format!("expected {what} duration (e.g. 500ms, 0.7s, 5m)"))),
        }
    }

    pub fn expect_int_in(&mut self, what: &str, min: i64, max: i64) -> Result<(i64, Pos), CompileError> {
        let (value, pos) = self.expect_int(what)?;
        if value < min || value > max {
            return Err(pos.err(format!("{what} must be {min}..={max}, got {value}")));
        }
        Ok((value, pos))
    }
}
