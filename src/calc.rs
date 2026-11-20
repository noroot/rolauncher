// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2025-2026 dmth

//! Inline calculator: evaluates simple arithmetic typed into the search field.
//!
//! Supports `+ - * / % ^`, parentheses, unary minus and decimals.
//! `^` is right-associative exponentiation, `%` is floating-point remainder.

const OPERATORS: &[char] = &['+', '-', '*', '/', '%', '^'];

/// A successfully evaluated expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Calculation {
    /// e.g. `"2 + 2 = 4"` — shown as the row title.
    pub display: String,
    /// e.g. `"4"` — copied to the clipboard.
    pub value: String,
}

/// Returns `Some` only when `input` looks like arithmetic and evaluates to a finite number.
pub fn calculate(input: &str) -> Option<Calculation> {
    let trimmed = input.trim();
    if !looks_like_math(trimmed) {
        return None;
    }
    let result = evaluate(trimmed)?;
    let value = format_number(result);
    Some(Calculation {
        display: format!("{trimmed} = {value}"),
        value,
    })
}

fn looks_like_math(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_digit())
        && s.chars().any(|c| OPERATORS.contains(&c))
        && s.chars()
            .all(|c| c.is_ascii_digit() || c.is_whitespace() || OPERATORS.contains(&c) || "().".contains(c))
}

/// Evaluates an arithmetic expression. `None` on syntax errors or non-finite results.
pub fn evaluate(expr: &str) -> Option<f64> {
    let tokens = tokenize(expr)?;
    let mut parser = Parser { tokens: &tokens, pos: 0 };
    let value = parser.expr()?;
    (parser.pos == tokens.len() && value.is_finite()).then_some(value)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Token {
    Num(f64),
    Op(char),
    LParen,
    RParen,
}

fn tokenize(s: &str) -> Option<Vec<Token>> {
    let chars: Vec<char> = s.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() => i += 1,
            '0'..='9' | '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                // Reject ".5", "5.", "1.2.3" — same strictness as the Swift version.
                if text.starts_with('.') || text.ends_with('.') || text.matches('.').count() > 1 {
                    return None;
                }
                tokens.push(Token::Num(text.parse().ok()?));
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            c if OPERATORS.contains(&c) => {
                tokens.push(Token::Op(c));
                i += 1;
            }
            _ => return None,
        }
    }
    Some(tokens)
}

/// Recursive-descent parser:
/// ```text
/// expr    := term (('+' | '-') term)*
/// term    := unary (('*' | '/' | '%') unary)*
/// unary   := '-' unary | power
/// power   := primary ('^' unary)?
/// primary := NUMBER | '(' expr ')'
/// ```
struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.peek();
        self.pos += 1;
        t
    }

    fn expr(&mut self) -> Option<f64> {
        let mut acc = self.term()?;
        while let Some(Token::Op(op @ ('+' | '-'))) = self.peek() {
            self.pos += 1;
            let rhs = self.term()?;
            acc = if op == '+' { acc + rhs } else { acc - rhs };
        }
        Some(acc)
    }

    fn term(&mut self) -> Option<f64> {
        let mut acc = self.unary()?;
        while let Some(Token::Op(op @ ('*' | '/' | '%'))) = self.peek() {
            self.pos += 1;
            let rhs = self.unary()?;
            acc = match op {
                '*' => acc * rhs,
                '/' => acc / rhs,
                _ => acc % rhs,
            };
        }
        Some(acc)
    }

    fn unary(&mut self) -> Option<f64> {
        if self.peek() == Some(Token::Op('-')) {
            self.pos += 1;
            return Some(-self.unary()?);
        }
        self.power()
    }

    fn power(&mut self) -> Option<f64> {
        let base = self.primary()?;
        if self.peek() == Some(Token::Op('^')) {
            self.pos += 1;
            let exp = self.unary()?;
            return Some(base.powf(exp));
        }
        Some(base)
    }

    fn primary(&mut self) -> Option<f64> {
        match self.next()? {
            Token::Num(n) => Some(n),
            Token::LParen => {
                let v = self.expr()?;
                (self.next()? == Token::RParen).then_some(v)
            }
            _ => None,
        }
    }
}

/// Integers print without a decimal point; fractions keep up to 10 digits.
pub fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    let s = format!("{n:.10}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(s: &str) -> Option<String> {
        calculate(s).map(|c| c.value)
    }

    #[test]
    fn basic_arithmetic() {
        assert_eq!(eval("2+2").as_deref(), Some("4"));
        assert_eq!(eval(" 2 + 3 * 4 ").as_deref(), Some("14"));
        assert_eq!(eval("(2+3)*4").as_deref(), Some("20"));
        assert_eq!(eval("10/4").as_deref(), Some("2.5"));
        assert_eq!(eval("1/3").as_deref(), Some("0.3333333333"));
        assert_eq!(eval("10%3").as_deref(), Some("1"));
        assert_eq!(eval("2^10").as_deref(), Some("1024"));
        assert_eq!(eval("2^3^2").as_deref(), Some("512")); // right-assoc
        assert_eq!(eval("-2^2").as_deref(), Some("-4"));
        assert_eq!(eval("-5+3").as_deref(), Some("-2"));
        assert_eq!(eval("2*-3").as_deref(), Some("-6"));
        assert_eq!(eval("1.5*2").as_deref(), Some("3"));
    }

    #[test]
    fn display_string() {
        assert_eq!(calculate(" 2 + 2 ").unwrap().display, "2 + 2 = 4");
    }

    #[test]
    fn rejects_non_math_and_incomplete_input() {
        for s in [
            "", "safari", "42", "2+", "+2", "2++2", "(2+3", "2+3)", "2(3)", ".5+1", "5.+1",
            "1.2.3+1", "1/0", "0/0", "()", "2 + x",
        ] {
            assert_eq!(calculate(s), None, "{s:?} should not evaluate");
        }
    }
}
