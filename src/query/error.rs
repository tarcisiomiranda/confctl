use std::fmt;

#[derive(Debug, Clone)]
pub struct QueryError {
    pub message: String,
    pub offset: Option<usize>,
}

impl QueryError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            offset: None,
        }
    }

    pub fn at(message: impl Into<String>, offset: usize) -> Self {
        Self {
            message: message.into(),
            offset: Some(offset),
        }
    }
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.offset {
            Some(off) => write!(f, "query error at {}: {}", off, self.message),
            None => write!(f, "query error: {}", self.message),
        }
    }
}

impl std::error::Error for QueryError {}

pub type Result<T> = std::result::Result<T, QueryError>;
