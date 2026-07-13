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
fn test_redact_masks_sensitive_keys_keeps_others() {
    let data = json!({
        "DATABASE_HOST": "192.0.2.9",
        "API_KEY": "sk-live-abc123",
        "postgres_password": "hunter2",
        "DEBUG": true
    });

    let redacted = redact_sensitive(&data);

    assert_eq!(redacted["DATABASE_HOST"], json!("192.0.2.9"));
    assert_eq!(redacted["DEBUG"], json!(true));
    assert_eq!(redacted["API_KEY"], json!("<redacted>"));
    assert_eq!(redacted["postgres_password"], json!("<redacted>"));
}

#[test]
fn test_redact_masks_pass_and_pwd_key_variants() {
    let data = json!({
        "DB_PASS": "hunter2",
        "REDIS_PWD": "hunter3",
        "DB_HOST": "192.0.2.9"
    });

    let redacted = redact_sensitive(&data);

    assert_eq!(redacted["DB_PASS"], json!("<redacted>"));
    assert_eq!(redacted["REDIS_PWD"], json!("<redacted>"));
    assert_eq!(redacted["DB_HOST"], json!("192.0.2.9"));
}

#[test]
fn test_redact_masks_secret_shaped_values_regardless_of_key() {
    let data = json!({
        "GITHUB_CLONE": "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "SESSION": "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.abc123",
        "AWS_ID": "AKIAIOSFODNN7EXAMPLE",
        "PEM": "-----BEGIN RSA PRIVATE KEY-----",
        "FRONTEND_URL": "https://example.com/app"
    });

    let redacted = redact_sensitive(&data);

    assert_eq!(redacted["GITHUB_CLONE"], json!("<redacted>"));
    assert_eq!(redacted["SESSION"], json!("<redacted>"));
    assert_eq!(redacted["AWS_ID"], json!("<redacted>"));
    assert_eq!(redacted["PEM"], json!("<redacted>"));
    assert_eq!(redacted["FRONTEND_URL"], json!("https://example.com/app"));
}

#[test]
fn test_redact_recurses_nested_objects_and_arrays() {
    let data = json!({
        "services": [
            {"name": "api", "auth_token": "t0ps3cret", "port": 8080}
        ],
        "secrets": {"stripe": "sk_live_x"}
    });

    let redacted = redact_sensitive(&data);

    assert_eq!(redacted["services"][0]["name"], json!("api"));
    assert_eq!(redacted["services"][0]["port"], json!(8080));
    assert_eq!(redacted["services"][0]["auth_token"], json!("<redacted>"));
    // A sensitive key masks its whole subtree, not just scalars.
    assert_eq!(redacted["secrets"], json!("<redacted>"));
}

#[test]
fn test_diff_values_env_added_removed_changed() {
    let left = json!({
        "DATA_DIR": "/srv/data",
        "POSTGRES_PASSWORD": "old-secret",
        "DOCKER_SOCK_GID": 989
    });
    let right = json!({
        "DATA_DIR": "./data",
        "POSTGRES_PASSWORD": "new-secret",
        "DATABASE_HOST": "192.0.2.9"
    });

    let changes = diff::diff_values(&left, &right);

    assert!(changes.iter().any(|change| matches!(
        change,
        diff::DiffChange::Changed { path, .. } if path == "DATA_DIR"
    )));
    assert!(changes.iter().any(|change| matches!(
        change,
        diff::DiffChange::Added { path, .. } if path == "DATABASE_HOST"
    )));
    assert!(changes.iter().any(|change| matches!(
        change,
        diff::DiffChange::Removed { path, .. } if path == "DOCKER_SOCK_GID"
    )));
}

#[test]
fn test_diff_values_nested_paths_and_array_indexes() {
    let left = json!({
        "services": {
            "api": {"image": "api:v1"},
            "caddy": {"ports": [80]}
        }
    });
    let right = json!({
        "services": {
            "api": {"image": "api:v2"},
            "caddy": {"ports": [80, 443]}
        }
    });

    let changes = diff::diff_values(&left, &right);

    assert!(changes.iter().any(|change| matches!(
        change,
        diff::DiffChange::Changed { path, .. } if path == "services.api.image"
    )));
    assert!(changes.iter().any(|change| matches!(
        change,
        diff::DiffChange::Added { path, .. } if path == "services.caddy.ports[1]"
    )));
}

#[test]
fn test_format_diff_masks_secrets_by_default() {
    let changes = vec![diff::DiffChange::Changed {
        path: "POSTGRES_PASSWORD".to_string(),
        old: json!("old-secret"),
        new: json!("new-secret"),
    }];

    let output = diff::format_structural_diff(&changes, false, false);

    assert!(output.contains("<secret changed>"));
    assert!(!output.contains("old-secret"));
    assert!(!output.contains("new-secret"));
}

#[test]
fn test_format_diff_can_show_secrets() {
    let changes = vec![diff::DiffChange::Changed {
        path: "POSTGRES_PASSWORD".to_string(),
        old: json!("old-secret"),
        new: json!("new-secret"),
    }];

    let output = diff::format_structural_diff(&changes, true, false);

    assert!(output.contains("old-secret"));
    assert!(output.contains("new-secret"));
}

#[test]
fn test_diff_values_no_changes() {
    let left = json!({"name": "confctl", "version": 1});
    let right = json!({"name": "confctl", "version": 1});

    assert!(diff::diff_values(&left, &right).is_empty());
}

#[test]
fn test_format_text_diff_marks_added_and_removed_lines() {
    let output = diff::format_text_diff("old\nsame\n", "same\nnew\n", false);

    assert!(output.contains("Text diff"));
    assert!(output.contains("- old"));
    assert!(output.contains("+ new"));
    assert!(output.contains("  same"));
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
fn test_auto_secret_name_is_dir_file_date_slug() {
    let name = vault::cli::auto_secret_name(".env");
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    assert!(name.ends_with(&today), "name should end with today's date: {name}");
    assert!(name.contains("env"), "name should carry the filename: {name}");
    assert!(
        name.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "name must stay in the [A-Za-z0-9_-] charset: {name}"
    );
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
