use super::*;
use serde_json::json;
use std::process::Command;

#[test]
fn test_resolve_simple_key() {
    let data = json!({"club": {"name": "Vasco da Gama", "founded": 1898}});
    let result = resolve_path(&data, "club.name").unwrap();
    assert_eq!(result, &json!("Vasco da Gama"));
}

#[test]
fn test_resolve_numeric_index() {
    let data = json!({"players": [{"name": "Edmundo"}, {"name": "Juninho Pernambucano"}]});
    let result = resolve_path(&data, "players.1.name").unwrap();
    assert_eq!(result, &json!("Juninho Pernambucano"));
}

#[test]
fn test_resolve_missing_key() {
    let data = json!({"club": {"name": "Vasco da Gama"}});
    let result = resolve_path(&data, "club.stadium");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Key not found"));
}

#[test]
fn test_resolve_index_out_of_bounds() {
    let data = json!({"titles": [1, 2, 3]});
    let result = resolve_path(&data, "titles.5");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("out of bounds"));
}

#[test]
fn test_resolve_scalar_traversal() {
    let data = json!({"name": "Vasco da Gama"});
    let result = resolve_path(&data, "name.something");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("scalar value"));
}

#[test]
fn test_format_string_no_quotes() {
    assert_eq!(format_value(&json!("Edmundo")), "Edmundo");
}

#[test]
fn test_format_number() {
    assert_eq!(format_value(&json!(9)), "9");
}

#[test]
fn test_format_bool() {
    assert_eq!(format_value(&json!(true)), "true");
}

#[test]
fn test_detect_format_no_extension_json() {
    let content = r#"{"club":"Vasco"}"#;
    let format = detect_format("response", content, None).unwrap();
    assert_eq!(format, Format::Json);
}

#[test]
fn test_detect_format_no_extension_toml() {
    let content = r#"club = "Vasco""#;
    let format = detect_format("response", content, None).unwrap();
    assert_eq!(format, Format::Toml);
}

#[test]
fn test_detect_format_forced_overrides_extension() {
    let content = r#"club: Vasco"#;
    let format = detect_format("response.json", content, Some(Format::Yaml)).unwrap();
    assert_eq!(format, Format::Yaml);
}

#[test]
fn test_resolve_input_no_file_uses_stdin_when_piped() {
    let (file, path) = resolve_input(None, None, false).unwrap();
    assert_eq!(file, "-");
    assert_eq!(path, None);
}

#[test]
fn test_resolve_input_single_arg_becomes_path_when_piped_and_file_missing() {
    let (file, path) = resolve_input(Some("geo.country".to_string()), None, false).unwrap();
    assert_eq!(file, "-");
    assert_eq!(path, Some("geo.country".to_string()));
}

#[test]
fn test_resolve_input_keeps_explicit_file_when_present() {
    let (file, path) =
        resolve_input(Some("testdata/config.json".to_string()), None, false).unwrap();
    assert_eq!(file, "testdata/config.json");
    assert_eq!(path, None);
}

#[test]
fn test_resolve_input_no_file_and_interactive_shows_tutorial() {
    let err = resolve_input(None, None, true).unwrap_err();
    assert!(err.to_string().contains("Mini tutorial"));
}

#[test]
#[ignore = "requires internet access to GitHub API"]
fn test_github_users_api_query() {
    let output = Command::new("curl")
        .args(["-fsSL", "https://api.github.com/users"])
        .output()
        .expect("failed to execute curl");

    assert!(
        output.status.success(),
        "curl failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let body = String::from_utf8(output.stdout).expect("GitHub API response is not valid UTF-8");
    let value = parse_content("stdin", &body, Some(Format::Json))
        .expect("failed to parse GitHub API response as JSON");
    let first_login = resolve_path(&value, "0.login").expect("path 0.login not found");

    match first_login {
        Value::String(login) => assert!(!login.is_empty(), "0.login should not be empty"),
        other => panic!("expected 0.login to be a string, got: {other}"),
    }
}
