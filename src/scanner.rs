use std::str::Chars;

use Token::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Token {
    Eof,
    Error,

    Plus,
    Minus,
    Star,
    Slash,
    Colon,
    Semicolon,
    Comma,
    Dot,
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
    Bang,
    BangEqual,
    Equal,
    EqualEqual,
    RightArrow,
    OpenParen,
    CloseParen,
    OpenBrace,
    CloseBrace,
    OpenBracket,
    CloseBracket,
    Identifier,

    NumberLiteral,
    StringLiteral,

    // Keywords
    Fn,
    Let,
    Return,
    Break,
    Continue,
    If,
    Else,
    While,
    For,
    Loop,
    True,
    False,
}

pub struct Scanner<'a> {
    start: &'a str,
    current: Chars<'a>,
    line: u32,
    error_message: &'a str,
}

impl<'a> Scanner<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            start: source,
            current: source.chars(),
            line: 0,
            error_message: "",
        }
    }

    pub fn scan_token(&mut self) -> Token {
        self.skip_whitespace();
        self.start = self.current.as_str();
        let Some(c) = self.current.next() else {
            return Eof;
        };
        match c {
            '+' => Plus,
            '-' => self.try_eat('>', RightArrow, Minus),
            '*' => Star,
            '/' => Slash,
            ':' => Colon,
            ';' => Semicolon,
            ',' => Comma,
            '.' => Dot,
            '>' => self.try_eat('=', GreaterEqual, Greater),
            '<' => self.try_eat('=', LessEqual, Less),
            '!' => self.try_eat('=', BangEqual, Bang),
            '=' => self.try_eat('=', EqualEqual, Equal),
            '(' => OpenParen,
            ')' => CloseParen,
            '{' => OpenBrace,
            '}' => CloseBrace,
            '[' => OpenBracket,
            ']' => CloseBracket,
            '"' => self.scan_string(),
            _ => {
                if c.is_ascii_digit() {
                    return self.scan_number();
                }
                if c.is_ascii_alphabetic() || c == '_' {
                    return self.scan_identifier();
                }
                self.error_message = "Unexpected character";
                Error
            }
        }
    }

    pub fn token_text(&self) -> &str {
        let len = self.start.len() - self.current.as_str().len();
        &self.start[..len]
    }

    pub fn line(&self) -> u32 {
        self.line
    }

    pub fn error_message(&self) -> &'a str {
        self.error_message
    }

    fn scan_string(&mut self) -> Token {
        while let Some(c) = self.current.next() {
            if c == '"' {
                return StringLiteral;
            }
        }
        self.error_message = "Unterminated string literal.";
        Error
    }

    fn scan_number(&mut self) -> Token {
        while let Some(c) = self.peek_char()
            && c.is_ascii_digit()
        {
            self.current.next();
        }

        if self.peek_char() == Some('.') {
            self.current.next();
            while let Some(c) = self.peek_char()
                && c.is_ascii_digit()
            {
                self.current.next();
            }
        }

        NumberLiteral
    }

    fn scan_identifier(&mut self) -> Token {
        while let Some(c) = self.peek_char()
            && (c.is_ascii_alphanumeric() || c == '_')
        {
            self.current.next();
        }

        let text = self.token_text();
        match text {
            "fn" => Fn,
            "let" => Let,
            "return" => Return,
            "break" => Break,
            "continue" => Continue,
            "if" => If,
            "else" => Else,
            "while" => While,
            "for" => For,
            "loop" => Loop,
            "true" => True,
            "false" => False,
            _ => Identifier,
        }
    }

    fn try_eat(&mut self, c: char, then_token: Token, else_token: Token) -> Token {
        if self.peek_char() == Some(c) {
            self.current.next();
            then_token
        } else {
            else_token
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek_char() {
            match c {
                ' ' | '\r' | '\t' => {}
                '\n' => self.line += 1,
                _ => break,
            }
            self.current.next();
        }
    }

    fn peek_char(&self) -> Option<char> {
        self.current.clone().next()
    }
}
