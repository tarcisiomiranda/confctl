use super::error::{QueryError, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Dot,
    Pipe,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    LParen,
    RParen,
    Colon,
    Comma,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    Alt,
    Ident(String),
    String(String),
    Number(String),
    Eof,
}

#[derive(Debug, Clone)]
pub struct Spanned {
    pub token: Token,
    pub start: usize,
}

pub fn tokenize(input: &str) -> Result<Vec<Spanned>> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let start = i;
        let c = bytes[i] as char;

        if c.is_whitespace() {
            i += 1;
            continue;
        }

        let token = match c {
            '.' => {
                i += 1;
                Token::Dot
            }
            '|' => {
                i += 1;
                Token::Pipe
            }
            '[' => {
                i += 1;
                Token::LBracket
            }
            ']' => {
                i += 1;
                Token::RBracket
            }
            '{' => {
                i += 1;
                Token::LBrace
            }
            '}' => {
                i += 1;
                Token::RBrace
            }
            '(' => {
                i += 1;
                Token::LParen
            }
            ')' => {
                i += 1;
                Token::RParen
            }
            ':' => {
                i += 1;
                Token::Colon
            }
            ',' => {
                i += 1;
                Token::Comma
            }
            '=' if bytes.get(i + 1) == Some(&b'=') => {
                i += 2;
                Token::EqEq
            }
            '!' if bytes.get(i + 1) == Some(&b'=') => {
                i += 2;
                Token::NotEq
            }
            '<' if bytes.get(i + 1) == Some(&b'=') => {
                i += 2;
                Token::Le
            }
            '<' => {
                i += 1;
                Token::Lt
            }
            '>' if bytes.get(i + 1) == Some(&b'=') => {
                i += 2;
                Token::Ge
            }
            '>' => {
                i += 1;
                Token::Gt
            }
            '/' if bytes.get(i + 1) == Some(&b'/') => {
                i += 2;
                Token::Alt
            }
            '"' => {
                i += 1;
                let mut s = String::new();
                let mut closed = false;
                while i < bytes.len() {
                    match bytes[i] as char {
                        '"' => {
                            i += 1;
                            closed = true;
                            break;
                        }
                        '\\' if i + 1 < bytes.len() => {
                            i += 1;
                            let esc = bytes[i] as char;
                            s.push(match esc {
                                'n' => '\n',
                                't' => '\t',
                                'r' => '\r',
                                '"' => '"',
                                '\\' => '\\',
                                other => other,
                            });
                            i += 1;
                        }
                        ch => {
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                if !closed {
                    return Err(QueryError::at("unterminated string", start));
                }
                Token::String(s)
            }
            '\'' => {
                return Err(QueryError::at(
                    "single-quoted strings are not supported; use double quotes",
                    start,
                ));
            }
            ch if ch.is_ascii_digit()
                || (ch == '-' && bytes.get(i + 1).is_some_and(|b| b.is_ascii_digit())) =>
            {
                let mut end = i + 1;
                while end < bytes.len() {
                    let c2 = bytes[end] as char;
                    if c2.is_ascii_digit()
                        || c2 == '.'
                        || c2 == 'e'
                        || c2 == 'E'
                        || c2 == '+'
                        || c2 == '-'
                    {
                        // keep going for scientific notation; stop if second `-` not after e
                        end += 1;
                    } else {
                        break;
                    }
                }
                // trim invalid trailing +-/e from overly greedy scan
                let mut num = &input[i..end];
                while num.len() > 1 {
                    let last = num.as_bytes()[num.len() - 1] as char;
                    if last == 'e' || last == 'E' || last == '+' || last == '-' {
                        num = &num[..num.len() - 1];
                        end -= 1;
                    } else {
                        break;
                    }
                }
                i = end;
                Token::Number(num.to_string())
            }
            ch if ch == '_' || ch.is_ascii_alphabetic() => {
                let mut end = i + 1;
                while end < bytes.len() {
                    let c2 = bytes[end] as char;
                    if c2 == '_' || c2.is_ascii_alphanumeric() {
                        end += 1;
                    } else {
                        break;
                    }
                }
                let ident = input[i..end].to_string();
                i = end;
                Token::Ident(ident)
            }
            other => {
                return Err(QueryError::at(
                    format!("unexpected character '{other}'"),
                    start,
                ));
            }
        };

        tokens.push(Spanned { token, start });
    }

    tokens.push(Spanned {
        token: Token::Eof,
        start: input.len(),
    });
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lex_path_and_pipe() {
        let toks = tokenize(".foo.bar | select(.x)").unwrap();
        assert!(toks.iter().any(|t| matches!(t.token, Token::Pipe)));
        assert!(toks
            .iter()
            .any(|t| matches!(&t.token, Token::Ident(s) if s == "select")));
    }

    #[test]
    fn lex_operators() {
        let toks = tokenize("== != <= >= //").unwrap();
        let kinds: Vec<_> = toks.iter().map(|t| &t.token).collect();
        assert!(kinds.contains(&&Token::EqEq));
        assert!(kinds.contains(&&Token::NotEq));
        assert!(kinds.contains(&&Token::Le));
        assert!(kinds.contains(&&Token::Ge));
        assert!(kinds.contains(&&Token::Alt));
    }

    #[test]
    fn lex_string() {
        let toks = tokenize(r#".["k"]"#).unwrap();
        assert!(toks
            .iter()
            .any(|t| matches!(&t.token, Token::String(s) if s == "k")));
    }
}
