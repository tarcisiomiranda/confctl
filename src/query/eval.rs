use serde_json::{Map, Value};

use super::ast::{BinOp, Expr, ObjectKey};
use super::error::{QueryError, Result};

pub fn eval(expr: &Expr, input: &Value) -> Result<Vec<Value>> {
    match expr {
        Expr::Identity => Ok(vec![input.clone()]),
        Expr::Literal(v) => Ok(vec![v.clone()]),
        Expr::Field { base, name } => {
            let bases = eval(base, input)?;
            let mut out = Vec::new();
            for b in bases {
                match b {
                    Value::Object(map) => match map.get(name) {
                        Some(v) => out.push(v.clone()),
                        None => {
                            return Err(QueryError::new(format!("key not found: '{name}'")));
                        }
                    },
                    Value::Null => {
                        return Err(QueryError::new(format!(
                            "cannot index null with field '{name}'"
                        )));
                    }
                    other => {
                        return Err(QueryError::new(format!(
                            "cannot index {} with field '{name}'",
                            type_name(&other)
                        )));
                    }
                }
            }
            Ok(out)
        }
        Expr::Index { base, index } => {
            let bases = eval(base, input)?;
            let mut out = Vec::new();
            for b in bases {
                let idxs = eval(index, input)?;
                if idxs.len() != 1 {
                    return Err(QueryError::new(
                        "index expression must yield exactly one value",
                    ));
                }
                let idx_val = &idxs[0];
                match (&b, idx_val) {
                    (Value::Array(arr), Value::Number(n)) => {
                        let i = number_to_isize(n)
                            .ok_or_else(|| QueryError::new(format!("invalid array index: {n}")))?;
                        let len = arr.len() as isize;
                        let resolved = if i < 0 { len + i } else { i };
                        if resolved < 0 || resolved as usize >= arr.len() {
                            return Err(QueryError::new(format!(
                                "array index {i} out of bounds (length {})",
                                arr.len()
                            )));
                        }
                        out.push(arr[resolved as usize].clone());
                    }
                    (Value::Object(map), Value::String(k)) => match map.get(k) {
                        Some(v) => out.push(v.clone()),
                        None => return Err(QueryError::new(format!("key not found: '{k}'"))),
                    },
                    (Value::Array(_), other) => {
                        return Err(QueryError::new(format!(
                            "array index must be number, got {}",
                            type_name(other)
                        )));
                    }
                    (other, _) => {
                        return Err(QueryError::new(format!(
                            "cannot index {}",
                            type_name(other)
                        )));
                    }
                }
            }
            Ok(out)
        }
        Expr::Iterate { base } => {
            let bases = eval(base, input)?;
            let mut out = Vec::new();
            for b in bases {
                match b {
                    Value::Array(arr) => out.extend(arr),
                    Value::Object(map) => out.extend(map.into_iter().map(|(_, v)| v)),
                    other => {
                        return Err(QueryError::new(format!(
                            "cannot iterate over {}",
                            type_name(&other)
                        )));
                    }
                }
            }
            Ok(out)
        }
        Expr::Pipe(stages) => {
            let mut current = vec![input.clone()];
            for stage in stages {
                let mut next = Vec::new();
                for item in current {
                    next.extend(eval(stage, &item)?);
                }
                current = next;
            }
            Ok(current)
        }
        Expr::BinOp { op, left, right } => eval_binop(*op, left, right, input),
        Expr::Not(inner) => {
            let vals = eval(inner, input)?;
            let mut out = Vec::new();
            for v in vals {
                out.push(Value::Bool(!is_truthy(&v)));
            }
            Ok(out)
        }
        Expr::Call { name, args } => eval_call(name, args, input),
        Expr::ArrayConstruct(None) => Ok(vec![Value::Array(vec![])]),
        Expr::ArrayConstruct(Some(inner)) => {
            // collect stream into one array
            let items = eval(inner, input)?;
            Ok(vec![Value::Array(items)])
        }
        Expr::ObjectConstruct(fields) => {
            let mut map = Map::new();
            for field in fields {
                let key = match &field.key {
                    ObjectKey::Ident(s) | ObjectKey::String(s) => s.clone(),
                };
                let value_expr = match &field.value {
                    Some(e) => e.clone(),
                    None => {
                        // shorthand {key} => {key: .key}
                        Expr::Field {
                            base: Box::new(Expr::Identity),
                            name: key.clone(),
                        }
                    }
                };
                let vals = eval(&value_expr, input)?;
                if vals.len() != 1 {
                    return Err(QueryError::new(format!(
                        "object value for '{key}' must yield exactly one value, got {}",
                        vals.len()
                    )));
                }
                map.insert(key, vals.into_iter().next().unwrap());
            }
            Ok(vec![Value::Object(map)])
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            let conds = eval(cond, input)?;
            // jq: if any condition is truthy, then; need single cond typically
            let truthy = conds.iter().any(is_truthy);
            if truthy {
                eval(then_branch, input)
            } else {
                eval(else_branch, input)
            }
        }
    }
}

fn eval_binop(op: BinOp, left: &Expr, right: &Expr, input: &Value) -> Result<Vec<Value>> {
    match op {
        BinOp::Alt => {
            let left_vals = eval(left, input)?;
            let useful: Vec<_> = left_vals
                .into_iter()
                .filter(|v| !matches!(v, Value::Null) && !matches!(v, Value::Bool(false)))
                .collect();
            if useful.is_empty() {
                eval(right, input)
            } else {
                Ok(useful)
            }
        }
        BinOp::And => {
            let left_vals = eval(left, input)?;
            let mut out = Vec::new();
            for lv in left_vals {
                if !is_truthy(&lv) {
                    out.push(lv);
                } else {
                    out.extend(eval(right, input)?);
                }
            }
            Ok(out)
        }
        BinOp::Or => {
            let left_vals = eval(left, input)?;
            let mut out = Vec::new();
            for lv in left_vals {
                if is_truthy(&lv) {
                    out.push(lv);
                } else {
                    out.extend(eval(right, input)?);
                }
            }
            Ok(out)
        }
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let ls = eval(left, input)?;
            let rs = eval(right, input)?;
            let mut out = Vec::new();
            for l in &ls {
                for r in &rs {
                    let result = match op {
                        BinOp::Eq => Value::Bool(json_eq(l, r)),
                        BinOp::Ne => Value::Bool(!json_eq(l, r)),
                        BinOp::Lt => {
                            Value::Bool(json_cmp(l, r).map(|o| o.is_lt()).unwrap_or(false))
                        }
                        BinOp::Le => {
                            Value::Bool(json_cmp(l, r).map(|o| o.is_le()).unwrap_or(false))
                        }
                        BinOp::Gt => {
                            Value::Bool(json_cmp(l, r).map(|o| o.is_gt()).unwrap_or(false))
                        }
                        BinOp::Ge => {
                            Value::Bool(json_cmp(l, r).map(|o| o.is_ge()).unwrap_or(false))
                        }
                        _ => unreachable!(),
                    };
                    out.push(result);
                }
            }
            Ok(out)
        }
    }
}

fn eval_call(name: &str, args: &[Expr], input: &Value) -> Result<Vec<Value>> {
    match name {
        "select" => {
            if args.len() != 1 {
                return Err(QueryError::new("select(expr) takes exactly one argument"));
            }
            let conds = eval(&args[0], input)?;
            if conds.iter().any(is_truthy) {
                Ok(vec![input.clone()])
            } else {
                Ok(vec![])
            }
        }
        "map" => {
            if args.len() != 1 {
                return Err(QueryError::new("map(expr) takes exactly one argument"));
            }
            match input {
                Value::Array(arr) => {
                    let mut out = Vec::new();
                    for item in arr {
                        let mapped = eval(&args[0], item)?;
                        // jq map collects first result per element, or all into array of arrays?
                        // jq: map(f) = [.[] | f] — collects all outputs of f per element as...
                        // actually each element's f stream is collected: if f yields one, array of those.
                        // If f yields multiple, they're all included in the outer array flattened?
                        // jq map(.): for [1,2] → [1,2]. For map(.[] ) on [[1,2],[3]] → [1,2,3].
                        // So it's [.[] | f] which flattens streams into one array.
                        out.extend(mapped);
                    }
                    Ok(vec![Value::Array(out)])
                }
                other => Err(QueryError::new(format!(
                    "map requires array input, got {}",
                    type_name(other)
                ))),
            }
        }
        "length" => {
            if !args.is_empty() {
                return Err(QueryError::new("length takes no arguments"));
            }
            let n = match input {
                Value::Array(a) => a.len() as i64,
                Value::Object(o) => o.len() as i64,
                Value::String(s) => s.chars().count() as i64,
                Value::Null => 0,
                other => {
                    return Err(QueryError::new(format!(
                        "length not supported for {}",
                        type_name(other)
                    )));
                }
            };
            Ok(vec![Value::Number(n.into())])
        }
        "keys" => {
            if !args.is_empty() {
                return Err(QueryError::new("keys takes no arguments"));
            }
            match input {
                Value::Object(map) => {
                    let mut keys: Vec<String> = map.keys().cloned().collect();
                    keys.sort();
                    Ok(vec![Value::Array(
                        keys.into_iter().map(Value::String).collect(),
                    )])
                }
                Value::Array(arr) => Ok(vec![Value::Array(
                    (0..arr.len())
                        .map(|i| Value::Number((i as i64).into()))
                        .collect(),
                )]),
                other => Err(QueryError::new(format!(
                    "keys not supported for {}",
                    type_name(other)
                ))),
            }
        }
        "values" => {
            if !args.is_empty() {
                return Err(QueryError::new("values takes no arguments"));
            }
            match input {
                Value::Object(map) => {
                    let mut keys: Vec<&String> = map.keys().collect();
                    keys.sort();
                    Ok(vec![Value::Array(
                        keys.into_iter()
                            .map(|k| map.get(k).unwrap().clone())
                            .collect(),
                    )])
                }
                Value::Array(arr) => Ok(vec![Value::Array(arr.clone())]),
                other => Err(QueryError::new(format!(
                    "values not supported for {}",
                    type_name(other)
                ))),
            }
        }
        "type" => {
            if !args.is_empty() {
                return Err(QueryError::new("type takes no arguments"));
            }
            Ok(vec![Value::String(type_name(input).to_string())])
        }
        "empty" => {
            if !args.is_empty() {
                return Err(QueryError::new("empty takes no arguments"));
            }
            Ok(vec![])
        }
        other => Err(QueryError::new(format!("unknown function '{other}'"))),
    }
}

fn is_truthy(v: &Value) -> bool {
    !matches!(v, Value::Null | Value::Bool(false))
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn json_eq(a: &Value, b: &Value) -> bool {
    a == b
}

fn json_cmp(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let xf = x.as_f64()?;
            let yf = y.as_f64()?;
            xf.partial_cmp(&yf)
        }
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

fn number_to_isize(n: &serde_json::Number) -> Option<isize> {
    if let Some(i) = n.as_i64() {
        return isize::try_from(i).ok();
    }
    if let Some(f) = n.as_f64() {
        if f.fract() == 0.0 && f >= isize::MIN as f64 && f <= isize::MAX as f64 {
            return Some(f as isize);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::parse::parse;
    use serde_json::json;

    fn run(expr: &str, input: Value) -> Vec<Value> {
        let e = parse(expr).unwrap();
        eval(&e, &input).unwrap()
    }

    #[test]
    fn identity_and_field() {
        let v = json!({"a": {"b": 1}});
        assert_eq!(run(".", v.clone()), vec![v.clone()]);
        assert_eq!(run(".a.b", v), vec![json!(1)]);
    }

    #[test]
    fn iterate_pipe() {
        let v = json!({"clubs": [{"name": "Vasco"}, {"name": "Arsenal"}]});
        assert_eq!(
            run(".clubs[] | .name", v),
            vec![json!("Vasco"), json!("Arsenal")]
        );
    }

    #[test]
    fn select_filter() {
        let v = json!([{"n": 1, "ok": true}, {"n": 2, "ok": false}, {"n": 3, "ok": true}]);
        assert_eq!(run(".[] | select(.ok) | .n", v), vec![json!(1), json!(3)]);
    }

    #[test]
    fn map_and_length() {
        let v = json!([1, 2, 3]);
        assert_eq!(run("length", v.clone()), vec![json!(3)]);
        assert_eq!(run("map(.)", v), vec![json!([1, 2, 3])]);
    }

    #[test]
    fn map_project() {
        let v = json!([{"x": 1}, {"x": 2}]);
        assert_eq!(run("map(.x)", v), vec![json!([1, 2])]);
    }

    #[test]
    fn object_shorthand() {
        let v = json!({"id": 7, "name": "a", "extra": true});
        assert_eq!(run("{id, name}", v), vec![json!({"id": 7, "name": "a"})]);
    }

    #[test]
    fn compare_and_if() {
        let v = json!({"n": 5});
        assert_eq!(
            run("if .n > 3 then \"big\" else \"small\" end", v),
            vec![json!("big")]
        );
    }

    #[test]
    fn alt_operator() {
        let v = json!({"a": null});
        assert_eq!(run(".a // 42", v), vec![json!(42)]);
    }

    #[test]
    fn array_construct() {
        let v = json!([1, 2, 3]);
        assert_eq!(run("[.[] | .]", v), vec![json!([1, 2, 3])]);
    }

    #[test]
    fn keys_sorted() {
        let v = json!({"b": 1, "a": 2});
        assert_eq!(run("keys", v), vec![json!(["a", "b"])]);
    }

    #[test]
    fn index_number() {
        let v = json!([10, 20, 30]);
        assert_eq!(run(".[1]", v), vec![json!(20)]);
    }

    #[test]
    fn string_key() {
        let v = json!({"my-key": 1});
        assert_eq!(run(r#".["my-key"]"#, v), vec![json!(1)]);
    }
}
