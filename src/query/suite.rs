//! Comprehensive query regression suite — path parity, stream, truthiness, errors.

#[cfg(test)]
mod tests {
    use crate::query::{self, eval, parse};
    use serde_json::{json, Value};

    fn run(expr: &str, input: Value) -> Vec<Value> {
        query::run(expr, &input).unwrap_or_else(|e| panic!("query `{expr}` failed: {e}"))
    }

    fn err(expr: &str, input: Value) -> String {
        query::run(expr, &input)
            .expect_err("expected error")
            .to_string()
    }

    fn parse_err(expr: &str) -> String {
        parse(expr).expect_err("expected parse error").to_string()
    }

    // ── path / identity ──────────────────────────────────────────────

    #[test]
    fn identity_returns_root() {
        let v = json!({"x": 1});
        assert_eq!(run(".", v.clone()), vec![v]);
    }

    #[test]
    fn deep_field_chain() {
        let v = json!({"a": {"b": {"c": "ok"}}});
        assert_eq!(run(".a.b.c", v), vec![json!("ok")]);
    }

    #[test]
    fn numeric_index_zero_and_last() {
        let v = json!([10, 20, 30]);
        assert_eq!(run(".[0]", v.clone()), vec![json!(10)]);
        assert_eq!(run(".[2]", v), vec![json!(30)]);
    }

    #[test]
    fn negative_array_index() {
        let v = json!([10, 20, 30]);
        assert_eq!(run(".[-1]", v.clone()), vec![json!(30)]);
        assert_eq!(run(".[-3]", v), vec![json!(10)]);
    }

    #[test]
    fn string_key_bracket_and_dot_string() {
        let v = json!({"my-key": 1, "x": 2});
        assert_eq!(run(r#".["my-key"]"#, v.clone()), vec![json!(1)]);
        // . "x" after identity via bracket only for special keys
        assert_eq!(run(r#".["x"]"#, v), vec![json!(2)]);
    }

    #[test]
    fn missing_key_is_error() {
        let msg = err(".nope", json!({"a": 1}));
        assert!(msg.contains("key not found"), "{msg}");
    }

    #[test]
    fn index_out_of_bounds_is_error() {
        let msg = err(".[5]", json!([1, 2]));
        assert!(msg.contains("out of bounds"), "{msg}");
    }

    #[test]
    fn negative_index_out_of_bounds_is_error() {
        let msg = err(".[-9]", json!([1, 2]));
        assert!(msg.contains("out of bounds"), "{msg}");
    }

    #[test]
    fn field_on_array_is_error() {
        let msg = err(".foo", json!([1, 2]));
        assert!(msg.contains("cannot index"), "{msg}");
    }

    #[test]
    fn field_on_null_is_error() {
        let msg = err(".foo", Value::Null);
        assert!(msg.contains("null"), "{msg}");
    }

    // ── iterate + pipe stream ────────────────────────────────────────

    #[test]
    fn iterate_array_yields_each_element() {
        assert_eq!(
            run(".[]", json!([1, 2, 3])),
            vec![json!(1), json!(2), json!(3)]
        );
    }

    #[test]
    fn iterate_object_yields_values() {
        let v = json!({"b": 2, "a": 1});
        let mut got = run(".[]", v);
        got.sort_by_key(|x| x.as_i64().unwrap());
        assert_eq!(got, vec![json!(1), json!(2)]);
    }

    #[test]
    fn iterate_scalar_is_error() {
        let msg = err(".[]", json!(42));
        assert!(msg.contains("cannot iterate"), "{msg}");
    }

    #[test]
    fn empty_array_iterate_yields_nothing() {
        assert!(run(".[]", json!([])).is_empty());
    }

    #[test]
    fn multi_stage_pipe_fans_out() {
        let v = json!({"xs": [{"ys": [1, 2]}, {"ys": [3]}]});
        assert_eq!(run(".xs[] | .ys[]", v), vec![json!(1), json!(2), json!(3)]);
    }

    #[test]
    fn pipe_with_empty_mid_stage_drops_all() {
        let v = json!([1, 2, 3]);
        assert!(run(".[] | empty", v).is_empty());
    }

    // ── select / truthiness ──────────────────────────────────────────

    #[test]
    fn select_keeps_truthy_only() {
        let v = json!([
            {"ok": true, "n": 1},
            {"ok": false, "n": 2},
            {"ok": null, "n": 3},
            {"ok": 0, "n": 4},
            {"ok": "", "n": 5},
            {"ok": [], "n": 6}
        ]);
        // jq: only false and null are falsy
        assert_eq!(
            run(".[] | select(.ok) | .n", v),
            vec![json!(1), json!(4), json!(5), json!(6)]
        );
    }

    #[test]
    fn select_false_and_null_drop() {
        assert!(run("select(false)", json!({"a": 1})).is_empty());
        assert!(run("select(null)", json!({"a": 1})).is_empty());
    }

    #[test]
    fn select_zero_and_empty_string_keep() {
        assert_eq!(run("select(0)", json!(true)), vec![json!(true)]);
        assert_eq!(run(r#"select("")"#, json!(true)), vec![json!(true)]);
    }

    #[test]
    fn not_inverts_truthiness() {
        assert_eq!(run("not", json!(false)), vec![json!(true)]);
        assert_eq!(run("not", json!(true)), vec![json!(false)]);
        assert_eq!(run("not", json!(null)), vec![json!(true)]);
        assert_eq!(run("not", json!(0)), vec![json!(false)]);
    }

    // ── compare / logic / if ─────────────────────────────────────────

    #[test]
    fn comparisons_numbers_and_strings() {
        assert_eq!(run(".n == 5", json!({"n": 5})), vec![json!(true)]);
        assert_eq!(run(".n != 5", json!({"n": 5})), vec![json!(false)]);
        assert_eq!(run(".n < 10", json!({"n": 5})), vec![json!(true)]);
        assert_eq!(run(".n <= 5", json!({"n": 5})), vec![json!(true)]);
        assert_eq!(run(".n > 5", json!({"n": 5})), vec![json!(false)]);
        assert_eq!(run(".n >= 5", json!({"n": 5})), vec![json!(true)]);
        assert_eq!(run(r#".s == "ab""#, json!({"s": "ab"})), vec![json!(true)]);
        assert_eq!(run(r#".s < "b""#, json!({"s": "a"})), vec![json!(true)]);
    }

    #[test]
    fn compare_incompatible_types_is_false_not_error() {
        assert_eq!(run(".n < \"x\"", json!({"n": 1})), vec![json!(false)]);
    }

    #[test]
    fn and_or_semantics() {
        // and: if left falsy, yield left; else right
        assert_eq!(run("false and 1", json!({})), vec![json!(false)]);
        assert_eq!(run("true and 1", json!({})), vec![json!(1)]);
        // or: if left truthy, yield left; else right
        assert_eq!(run("0 or 99", json!({})), vec![json!(0)]);
        assert_eq!(run("false or 99", json!({})), vec![json!(99)]);
        assert_eq!(run("null or \"x\"", json!({})), vec![json!("x")]);
    }

    #[test]
    fn if_then_else() {
        assert_eq!(
            run("if .n > 0 then \"pos\" else \"non\" end", json!({"n": 1})),
            vec![json!("pos")]
        );
        // condition is `.n > 0` → false when n == 0
        assert_eq!(
            run("if .n > 0 then \"pos\" else \"non\" end", json!({"n": 0})),
            vec![json!("non")]
        );
        assert_eq!(
            run("if .n > 0 then \"pos\" else \"non\" end", json!({"n": -1})),
            vec![json!("non")]
        );
    }

    #[test]
    fn if_condition_uses_compare_result() {
        // .n == 0 is false when n is 0? n==0 → true
        assert_eq!(
            run(
                "if .n == 0 then \"zero\" else \"other\" end",
                json!({"n": 0})
            ),
            vec![json!("zero")]
        );
    }

    // ── alternative // ───────────────────────────────────────────────

    #[test]
    fn alt_replaces_null_and_false() {
        assert_eq!(run(".a // 1", json!({"a": null})), vec![json!(1)]);
        assert_eq!(run(".a // 1", json!({"a": false})), vec![json!(1)]);
        assert_eq!(run(".a // 1", json!({"a": 0})), vec![json!(0)]);
        assert_eq!(run(".a // 1", json!({"a": ""})), vec![json!("")]);
    }

    #[test]
    fn alt_missing_key_errors_not_fallback() {
        // field access on missing key errors before //
        let msg = err(".missing // 1", json!({"a": 1}));
        assert!(msg.contains("key not found"), "{msg}");
    }

    // ── construct ────────────────────────────────────────────────────

    #[test]
    fn object_shorthand_and_explicit() {
        let v = json!({"id": 1, "name": "x", "role": "admin"});
        assert_eq!(
            run("{id, name}", v.clone()),
            vec![json!({"id": 1, "name": "x"})]
        );
        assert_eq!(
            run(r#"{id: .id, label: .name}"#, v),
            vec![json!({"id": 1, "label": "x"})]
        );
    }

    #[test]
    fn empty_object_and_array_literals() {
        assert_eq!(run("{}", json!(null)), vec![json!({})]);
        assert_eq!(run("[]", json!(null)), vec![json!([])]);
    }

    #[test]
    fn array_construct_collects_stream() {
        let v = json!([1, 2, 3]);
        assert_eq!(run("[.[] | .]", v), vec![json!([1, 2, 3])]);
    }

    #[test]
    fn nested_object_in_pipe() {
        let v = json!([{"id": 1, "n": "a"}, {"id": 2, "n": "b"}]);
        assert_eq!(
            run(".[] | {id, name: .n}", v),
            vec![json!({"id": 1, "name": "a"}), json!({"id": 2, "name": "b"})]
        );
    }

    // ── builtins ─────────────────────────────────────────────────────

    #[test]
    fn length_array_object_string_null() {
        assert_eq!(run("length", json!([1, 2, 3])), vec![json!(3)]);
        assert_eq!(run("length", json!({"a": 1, "b": 2})), vec![json!(2)]);
        assert_eq!(run("length", json!("abç")), vec![json!(3)]); // chars not bytes
        assert_eq!(run("length", json!(null)), vec![json!(0)]);
    }

    #[test]
    fn length_on_number_is_error() {
        let msg = err("length", json!(3));
        assert!(msg.contains("length not supported"), "{msg}");
    }

    #[test]
    fn keys_object_sorted_and_array_indices() {
        assert_eq!(
            run("keys", json!({"b": 1, "a": 2})),
            vec![json!(["a", "b"])]
        );
        assert_eq!(run("keys", json!([true, false])), vec![json!([0, 1])]);
    }

    #[test]
    fn values_object_sorted_by_key() {
        assert_eq!(run("values", json!({"b": 2, "a": 1})), vec![json!([1, 2])]);
    }

    #[test]
    fn type_builtin() {
        assert_eq!(run("type", json!(null)), vec![json!("null")]);
        assert_eq!(run("type", json!(true)), vec![json!("boolean")]);
        assert_eq!(run("type", json!(1)), vec![json!("number")]);
        assert_eq!(run("type", json!("s")), vec![json!("string")]);
        assert_eq!(run("type", json!([])), vec![json!("array")]);
        assert_eq!(run("type", json!({})), vec![json!("object")]);
    }

    #[test]
    fn map_projects_and_flattens_streams() {
        let v = json!([[1, 2], [3]]);
        assert_eq!(run("map(.[])", v), vec![json!([1, 2, 3])]);
    }

    #[test]
    fn map_on_object_is_error() {
        let msg = err("map(.)", json!({"a": 1}));
        assert!(msg.contains("map requires array"), "{msg}");
    }

    #[test]
    fn unknown_function_is_error() {
        let msg = err("explode", json!(1));
        assert!(msg.contains("unknown function"), "{msg}");
    }

    #[test]
    fn select_arity_error() {
        let msg = err("select()", json!(1));
        // select() is Call with empty args - parse allows it
        assert!(msg.contains("select"), "{msg}");
    }

    // ── parse errors ─────────────────────────────────────────────────

    #[test]
    fn parse_rejects_trailing_junk() {
        let msg = parse_err(".foo bar");
        assert!(msg.contains("unexpected"), "{msg}");
    }

    #[test]
    fn parse_rejects_unterminated_string() {
        let msg = parse_err(r#".["abc"#);
        assert!(msg.contains("unterminated"), "{msg}");
    }

    #[test]
    fn parse_rejects_incomplete_if() {
        let msg = parse_err("if .x then 1");
        assert!(msg.contains("expected"), "{msg}");
    }

    #[test]
    fn parse_rejects_unbalanced_paren() {
        let msg = parse_err("(.foo");
        assert!(msg.contains("expected"), "{msg}");
    }

    #[test]
    fn parse_rejects_single_quotes() {
        let msg = parse_err(".['x']");
        assert!(msg.contains("single-quoted"), "{msg}");
    }

    #[test]
    fn parse_empty_input_is_error() {
        assert!(parse("").is_err());
        assert!(parse("   ").is_err());
    }

    // ── literals ─────────────────────────────────────────────────────

    #[test]
    fn literals_ignore_input() {
        assert_eq!(run("null", json!(1)), vec![json!(null)]);
        assert_eq!(run("true", json!(1)), vec![json!(true)]);
        assert_eq!(run("false", json!(1)), vec![json!(false)]);
        assert_eq!(run("42", json!(1)), vec![json!(42)]);
        assert_eq!(run(r#""hi""#, json!(1)), vec![json!("hi")]);
        assert_eq!(run("-3", json!(1)), vec![json!(-3)]);
    }

    // ── fixture: testdata/config.json parity ──────────────────────────

    #[test]
    fn fixture_clubs_length_and_names() {
        let data = load_fixture();
        assert_eq!(run(".clubs | length", data.clone()), vec![json!(4)]);
        let names = run(".clubs[] | .name", data.clone());
        assert_eq!(names.len(), 4);
        assert_eq!(names[0], json!("Club de Regatas Vasco da Gama"));
        assert_eq!(names[1], json!("Arsenal FC"));
    }

    #[test]
    fn fixture_filter_spain_clubs() {
        let data = load_fixture();
        let names = run(r#".clubs[] | select(.country == "Spain") | .name"#, data);
        assert_eq!(names, vec![json!("Real Madrid CF"), json!("FC Barcelona")]);
    }

    #[test]
    fn fixture_nested_players_stream() {
        let data = load_fixture();
        let players = run(".clubs[0].players[] | .name", data);
        assert_eq!(
            players,
            vec![json!("Edmundo"), json!("Juninho Pernambucano")]
        );
    }

    #[test]
    fn fixture_object_project_club() {
        let data = load_fixture();
        let got = run(".clubs[0] | {name, country}", data);
        assert_eq!(
            got,
            vec![json!({
                "name": "Club de Regatas Vasco da Gama",
                "country": "Brazil"
            })]
        );
    }

    #[test]
    fn fixture_titles_keys() {
        let data = load_fixture();
        let keys = run(".clubs[2].titles | keys", data);
        assert_eq!(keys, vec![json!(["champions_league", "la_liga"])]);
    }

    #[test]
    fn fixture_parity_dotted_path_vs_query() {
        // Same leaf via legacy resolve_path and -q expression
        let data = load_fixture();
        let via_q = run(".clubs[0].players[1].name", data.clone());
        assert_eq!(via_q, vec![json!("Juninho Pernambucano")]);

        let via_path = crate::resolve_path(&data, "clubs.0.players.1.name").unwrap();
        assert_eq!(via_path, &json!("Juninho Pernambucano"));
    }

    #[test]
    fn fixture_midfielders_across_clubs() {
        let data = load_fixture();
        let names = run(
            r#".clubs[] | .players[] | select(.position == "midfielder") | .name"#,
            data,
        );
        assert!(names.contains(&json!("Juninho Pernambucano")));
        assert!(names.contains(&json!("Martin Odegaard")));
        assert!(names.contains(&json!("Jude Bellingham")));
        assert_eq!(names.len(), 4); // one midfielder per club in fixture
    }

    fn load_fixture() -> Value {
        crate::parse_file("testdata/config.json", Some(crate::Format::Json))
            .expect("testdata/config.json")
    }

    // ── eval helper: parse then eval for internal consistency ────────

    #[test]
    fn parse_eval_roundtrip_complex() {
        let expr = r#".[] | select(.n >= 2) | {n, doubled: .n}"#;
        let e = parse(expr).unwrap();
        let out = eval(&e, &json!([{"n": 1}, {"n": 2}, {"n": 3}])).unwrap();
        assert_eq!(
            out,
            vec![json!({"n": 2, "doubled": 2}), json!({"n": 3, "doubled": 3})]
        );
    }
}
