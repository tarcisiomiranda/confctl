use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Alt,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Identity,
    Literal(Value),
    Field {
        base: Box<Expr>,
        name: String,
    },
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    Iterate {
        base: Box<Expr>,
    },
    Pipe(Vec<Expr>),
    BinOp {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Not(Box<Expr>),
    Call {
        name: String,
        args: Vec<Expr>,
    },
    ArrayConstruct(Option<Box<Expr>>),
    ObjectConstruct(Vec<ObjectField>),
    If {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectField {
    pub key: ObjectKey,
    pub value: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObjectKey {
    Ident(String),
    String(String),
}
