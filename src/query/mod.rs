mod ast;
mod error;
mod eval;
mod lex;
mod parse;

#[cfg(test)]
mod suite;

pub use error::QueryError;

#[cfg(test)]
pub(crate) use eval::eval;
#[cfg(test)]
pub(crate) use parse::parse;

use serde_json::Value;

/// Parse and evaluate a jq-subset expression against `input`.
/// Returns zero or more result values (stream semantics).
pub fn run(source: &str, input: &Value) -> Result<Vec<Value>, QueryError> {
    let expr = parse::parse(source)?;
    eval::eval(&expr, input)
}
