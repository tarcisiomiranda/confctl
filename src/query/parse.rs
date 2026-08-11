use serde_json::Value;

use super::ast::{BinOp, Expr, ObjectField, ObjectKey};
use super::error::{QueryError, Result};
use super::lex::{tokenize, Spanned, Token};

pub fn parse(source: &str) -> Result<Expr> {
    let tokens = tokenize(source)?;
    let mut parser = Parser {
        tokens: &tokens,
        pos: 0,
    };
    let expr = parser.parse_expr()?;
    parser.expect_eof()?;
    Ok(expr)
}

struct Parser<'a> {
    tokens: &'a [Spanned],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos].token
    }

    fn peek_start(&self) -> usize {
        self.tokens[self.pos].start
    }

    fn bump(&mut self) -> &Token {
        let t = &self.tokens[self.pos].token;
        if !matches!(t, Token::Eof) {
            self.pos += 1;
        }
        &self.tokens[self.pos.saturating_sub(1)].token
    }

    fn expect_eof(&self) -> Result<()> {
        if matches!(self.peek(), Token::Eof) {
            Ok(())
        } else {
            Err(QueryError::at(
                format!("unexpected token {:?}", self.peek()),
                self.peek_start(),
            ))
        }
    }

    fn parse_expr(&mut self) -> Result<Expr> {
        self.parse_pipe()
    }

    fn parse_pipe(&mut self) -> Result<Expr> {
        let mut parts = vec![self.parse_alt()?];
        while matches!(self.peek(), Token::Pipe) {
            self.bump();
            parts.push(self.parse_alt()?);
        }
        if parts.len() == 1 {
            Ok(parts.pop().unwrap())
        } else {
            Ok(Expr::Pipe(parts))
        }
    }

    fn parse_alt(&mut self) -> Result<Expr> {
        let mut left = self.parse_or()?;
        while matches!(self.peek(), Token::Alt) {
            self.bump();
            let right = self.parse_or()?;
            left = Expr::BinOp {
                op: BinOp::Alt,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_or(&mut self) -> Result<Expr> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Token::Ident(s) if s == "or") {
            self.bump();
            let right = self.parse_and()?;
            left = Expr::BinOp {
                op: BinOp::Or,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr> {
        let mut left = self.parse_compare()?;
        while matches!(self.peek(), Token::Ident(s) if s == "and") {
            self.bump();
            let right = self.parse_compare()?;
            left = Expr::BinOp {
                op: BinOp::And,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_compare(&mut self) -> Result<Expr> {
        let left = self.parse_unary()?;
        let op = match self.peek() {
            Token::EqEq => Some(BinOp::Eq),
            Token::NotEq => Some(BinOp::Ne),
            Token::Lt => Some(BinOp::Lt),
            Token::Le => Some(BinOp::Le),
            Token::Gt => Some(BinOp::Gt),
            Token::Ge => Some(BinOp::Ge),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let right = self.parse_unary()?;
            Ok(Expr::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            })
        } else {
            Ok(left)
        }
    }

    fn parse_unary(&mut self) -> Result<Expr> {
        if matches!(self.peek(), Token::Ident(s) if s == "not") {
            self.bump();
            // Bare `not` is a filter on input (jq-style). `not <expr>` negates expr.
            if self.can_start_atom() {
                let inner = self.parse_unary()?;
                return Ok(Expr::Not(Box::new(inner)));
            }
            return Ok(Expr::Not(Box::new(Expr::Identity)));
        }
        self.parse_postfix()
    }

    fn can_start_atom(&self) -> bool {
        matches!(
            self.peek(),
            Token::Dot
                | Token::Number(_)
                | Token::String(_)
                | Token::Ident(_)
                | Token::LParen
                | Token::LBracket
                | Token::LBrace
        )
    }

    fn parse_postfix(&mut self) -> Result<Expr> {
        let mut expr = self.parse_atom()?;
        loop {
            match self.peek() {
                Token::Dot => {
                    self.bump();
                    match self.peek().clone() {
                        Token::Ident(name) => {
                            self.bump();
                            expr = Expr::Field {
                                base: Box::new(expr),
                                name,
                            };
                        }
                        Token::String(name) => {
                            self.bump();
                            expr = Expr::Field {
                                base: Box::new(expr),
                                name,
                            };
                        }
                        Token::LBracket => {
                            // .[ ...] handled as trailer without consuming extra dot semantics
                            // Actually `.` then `[` means index/iterate on current expr
                            expr = self.parse_bracket_trailer(expr)?;
                        }
                        other => {
                            return Err(QueryError::at(
                                format!("expected field name after '.', got {other:?}"),
                                self.peek_start(),
                            ));
                        }
                    }
                }
                Token::LBracket => {
                    expr = self.parse_bracket_trailer(expr)?;
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_bracket_trailer(&mut self, base: Expr) -> Result<Expr> {
        // consume '['
        if !matches!(self.peek(), Token::LBracket) {
            return Err(QueryError::at("expected '['", self.peek_start()));
        }
        self.bump();
        if matches!(self.peek(), Token::RBracket) {
            self.bump();
            return Ok(Expr::Iterate {
                base: Box::new(base),
            });
        }
        // string key .["k"] or index
        if let Token::String(name) = self.peek().clone() {
            self.bump();
            if !matches!(self.peek(), Token::RBracket) {
                return Err(QueryError::at(
                    "expected ']' after string key",
                    self.peek_start(),
                ));
            }
            self.bump();
            return Ok(Expr::Field {
                base: Box::new(base),
                name,
            });
        }
        let index = self.parse_expr()?;
        if !matches!(self.peek(), Token::RBracket) {
            return Err(QueryError::at("expected ']'", self.peek_start()));
        }
        self.bump();
        Ok(Expr::Index {
            base: Box::new(base),
            index: Box::new(index),
        })
    }

    fn parse_atom(&mut self) -> Result<Expr> {
        match self.peek().clone() {
            Token::Dot => {
                self.bump();
                // `.` alone or start of path; trailers applied in parse_postfix
                // But `.foo` is Dot + Ident as trailer — postfix handles after atom returns Identity
                // However parse_postfix only loops trailers AFTER atom. Atom is `.`, then
                // postfix sees Dot again for `.foo`?
                // Problem: `.foo` tokens = Dot, Ident — atom consumes first Dot → Identity,
                // then postfix sees Ident not Dot. Need to handle field after bare dot in atom
                // OR treat Ident after Identity start specially.
                //
                // Fix: after consuming `.`, if next is Ident/String/LBracket, apply trailers here
                // until no more, then return. Actually parse_postfix will handle LBracket and Dot.
                // For `.foo`, after Identity, peek is Ident — need Field without extra Dot.
                //
                // jq: `.foo` = field access. Grammar often: `.` field*
                // where field = `.` IDENT | `[` ...
                //
                // So after initial `.`, optional trailers that may start with `.` OR for first
                // field, IDENT directly without second dot.
                let mut expr = Expr::Identity;
                // first field may be bare ident after the opening dot
                if let Token::Ident(name) = self.peek().clone() {
                    self.bump();
                    expr = Expr::Field {
                        base: Box::new(expr),
                        name,
                    };
                } else if let Token::String(name) = self.peek().clone() {
                    self.bump();
                    expr = Expr::Field {
                        base: Box::new(expr),
                        name,
                    };
                } else if matches!(self.peek(), Token::LBracket) {
                    expr = self.parse_bracket_trailer(expr)?;
                }
                Ok(expr)
            }
            Token::Number(n) => {
                self.bump();
                let value = parse_number(&n, self.tokens[self.pos.saturating_sub(1)].start)?;
                Ok(Expr::Literal(value))
            }
            Token::String(s) => {
                self.bump();
                Ok(Expr::Literal(Value::String(s)))
            }
            Token::Ident(name) => {
                self.bump();
                match name.as_str() {
                    "null" => Ok(Expr::Literal(Value::Null)),
                    "true" => Ok(Expr::Literal(Value::Bool(true))),
                    "false" => Ok(Expr::Literal(Value::Bool(false))),
                    "if" => self.parse_if(),
                    other => {
                        // function call or bare builtin
                        if matches!(self.peek(), Token::LParen) {
                            self.bump();
                            let mut args = Vec::new();
                            if !matches!(self.peek(), Token::RParen) {
                                loop {
                                    args.push(self.parse_expr()?);
                                    if matches!(self.peek(), Token::Comma) {
                                        self.bump();
                                        continue;
                                    }
                                    break;
                                }
                            }
                            if !matches!(self.peek(), Token::RParen) {
                                return Err(QueryError::at("expected ')'", self.peek_start()));
                            }
                            self.bump();
                            Ok(Expr::Call {
                                name: other.to_string(),
                                args,
                            })
                        } else {
                            Ok(Expr::Call {
                                name: other.to_string(),
                                args: vec![],
                            })
                        }
                    }
                }
            }
            Token::LParen => {
                self.bump();
                let inner = self.parse_expr()?;
                if !matches!(self.peek(), Token::RParen) {
                    return Err(QueryError::at("expected ')'", self.peek_start()));
                }
                self.bump();
                Ok(inner)
            }
            Token::LBracket => {
                self.bump();
                if matches!(self.peek(), Token::RBracket) {
                    self.bump();
                    return Ok(Expr::ArrayConstruct(None));
                }
                let inner = self.parse_expr()?;
                if !matches!(self.peek(), Token::RBracket) {
                    return Err(QueryError::at("expected ']'", self.peek_start()));
                }
                self.bump();
                Ok(Expr::ArrayConstruct(Some(Box::new(inner))))
            }
            Token::LBrace => self.parse_object(),
            other => Err(QueryError::at(
                format!("unexpected token {other:?}"),
                self.peek_start(),
            )),
        }
    }

    fn parse_if(&mut self) -> Result<Expr> {
        // `if` already consumed
        let cond = self.parse_expr()?;
        match self.peek() {
            Token::Ident(s) if s == "then" => {
                self.bump();
            }
            _ => return Err(QueryError::at("expected 'then'", self.peek_start())),
        }
        let then_branch = self.parse_expr()?;
        match self.peek() {
            Token::Ident(s) if s == "else" => {
                self.bump();
            }
            _ => return Err(QueryError::at("expected 'else'", self.peek_start())),
        }
        let else_branch = self.parse_expr()?;
        match self.peek() {
            Token::Ident(s) if s == "end" => {
                self.bump();
            }
            _ => return Err(QueryError::at("expected 'end'", self.peek_start())),
        }
        Ok(Expr::If {
            cond: Box::new(cond),
            then_branch: Box::new(then_branch),
            else_branch: Box::new(else_branch),
        })
    }

    fn parse_object(&mut self) -> Result<Expr> {
        // consume '{'
        self.bump();
        let mut fields = Vec::new();
        if matches!(self.peek(), Token::RBrace) {
            self.bump();
            return Ok(Expr::ObjectConstruct(fields));
        }
        loop {
            let key = match self.peek().clone() {
                Token::Ident(name) => {
                    self.bump();
                    ObjectKey::Ident(name)
                }
                Token::String(name) => {
                    self.bump();
                    ObjectKey::String(name)
                }
                _ => {
                    return Err(QueryError::at("expected object key", self.peek_start()));
                }
            };
            let value = if matches!(self.peek(), Token::Colon) {
                self.bump();
                Some(self.parse_expr()?)
            } else {
                // shorthand only for Ident keys
                match &key {
                    ObjectKey::Ident(_) => None,
                    ObjectKey::String(_) => {
                        return Err(QueryError::at(
                            "string object key requires ': value'",
                            self.peek_start(),
                        ));
                    }
                }
            };
            fields.push(ObjectField { key, value });
            if matches!(self.peek(), Token::Comma) {
                self.bump();
                if matches!(self.peek(), Token::RBrace) {
                    break;
                }
                continue;
            }
            break;
        }
        if !matches!(self.peek(), Token::RBrace) {
            return Err(QueryError::at("expected '}'", self.peek_start()));
        }
        self.bump();
        Ok(Expr::ObjectConstruct(fields))
    }
}

fn parse_number(n: &str, offset: usize) -> Result<Value> {
    if let Ok(i) = n.parse::<i64>() {
        return Ok(Value::Number(i.into()));
    }
    if let Ok(f) = n.parse::<f64>() {
        if let Some(num) = serde_json::Number::from_f64(f) {
            return Ok(Value::Number(num));
        }
    }
    Err(QueryError::at(format!("invalid number '{n}'"), offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_field_path() {
        let e = parse(".foo.bar").unwrap();
        match e {
            Expr::Field { name, base } => {
                assert_eq!(name, "bar");
                match *base {
                    Expr::Field { name, .. } => assert_eq!(name, "foo"),
                    other => panic!("{other:?}"),
                }
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_iterate_and_pipe() {
        let e = parse(".clubs[] | .name").unwrap();
        assert!(matches!(e, Expr::Pipe(_)));
    }

    #[test]
    fn parse_select() {
        let e = parse("select(.active)").unwrap();
        match e {
            Expr::Call { name, args } => {
                assert_eq!(name, "select");
                assert_eq!(args.len(), 1);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_object_shorthand() {
        let e = parse("{id, name}").unwrap();
        match e {
            Expr::ObjectConstruct(fields) => assert_eq!(fields.len(), 2),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_if() {
        let e = parse("if .x then 1 else 0 end").unwrap();
        assert!(matches!(e, Expr::If { .. }));
    }
}
