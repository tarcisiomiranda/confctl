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
    assert_eq!(format_value_with(&json!("Edmundo"), false), "Edmundo");
}

#[test]
fn test_format_number() {
    assert_eq!(format_value_with(&json!(9), false), "9");
}

#[test]
fn test_format_bool() {
    assert_eq!(format_value_with(&json!(true), false), "true");
}

#[test]
fn test_format_compact_is_single_line_json() {
    let data = json!({"a": {"b": [1, 2]}, "c": "x"});

    let compact = format_value_with(&data, true);
    assert_eq!(compact, r#"{"a":{"b":[1,2]},"c":"x"}"#);
    assert!(!compact.contains('\n'));

    // Scalars are unaffected by compact mode.
    assert_eq!(format_value_with(&json!("plain"), true), "plain");

    // Default stays pretty-printed.
    assert!(format_value_with(&data, false).contains('\n'));
}

#[test]
fn test_redact_masks_sensitive_keys_keeps_others() {
    let data = json!({
        "DATABASE_HOST": "192.0.2.9",
        "API_KEY": "sk-live-abc123",
        "postgres_password": "hunter2",
        "DEBUG": true
    });

    let redacted = redact_sensitive(&data, 0);

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

    let redacted = redact_sensitive(&data, 0);

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

    let redacted = redact_sensitive(&data, 0);

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

    let redacted = redact_sensitive(&data, 0);

    assert_eq!(redacted["services"][0]["name"], json!("api"));
    assert_eq!(redacted["services"][0]["port"], json!(8080));
    assert_eq!(redacted["services"][0]["auth_token"], json!("<redacted>"));
    // A sensitive key masks its whole subtree, not just scalars.
    assert_eq!(redacted["secrets"], json!("<redacted>"));
}

#[test]
fn test_redact_masks_postgres_and_other_db_urls() {
    let data = json!({
        "DATABASE_URL": "postgres://admin:s3cretpass@db.example.com:5432/app",
        "PG": "postgresql://u:p@localhost/db",
        "MONGO": "mongodb+srv://user:pwd@cluster0.example.mongodb.net/app",
        "REDIS": "redis://:cachepass@127.0.0.1:6379/0",
        "JDBC": "jdbc:postgresql://admin:s3cret@db:5432/app",
        // No password → not treated as a secret-shaped value.
        "LOCAL_PG": "postgres://localhost:5432/app",
        "USER_ONLY": "postgres://admin@localhost/app",
        "HTTPS": "https://example.com/path"
    });

    let redacted = redact_sensitive(&data, 0);

    assert_eq!(
        redacted["DATABASE_URL"],
        json!("postgres://admin:<redacted>@db.example.com:5432/app")
    );
    assert_eq!(
        redacted["PG"],
        json!("postgresql://u:<redacted>@localhost/db")
    );
    assert_eq!(
        redacted["MONGO"],
        json!("mongodb+srv://user:<redacted>@cluster0.example.mongodb.net/app")
    );
    assert_eq!(
        redacted["REDIS"],
        json!("redis://:<redacted>@127.0.0.1:6379/0")
    );
    assert_eq!(
        redacted["JDBC"],
        json!("jdbc:postgresql://admin:<redacted>@db:5432/app")
    );
    assert_eq!(redacted["LOCAL_PG"], json!("postgres://localhost:5432/app"));
    assert_eq!(
        redacted["USER_ONLY"],
        json!("postgres://admin@localhost/app")
    );
    assert_eq!(redacted["HTTPS"], json!("https://example.com/path"));
}

#[test]
fn test_redact_percent_shows_start_and_end() {
    // 20 chars → 20% keep = 4 chars each side.
    let data = json!({
        "API_KEY": "abcdefghijklmnopqrst",
        "DB_PASS": "supersecretpassword!!"
    });

    let redacted = redact_sensitive(&data, 20);

    assert_eq!(redacted["API_KEY"], json!("abcd<redacted>qrst"));
    // "supersecretpassword!!" is 22 chars → 20% = 4
    assert_eq!(redacted["DB_PASS"], json!("supe<redacted>rd!!"));
}

#[test]
fn test_redact_percent_on_db_url_masks_only_password() {
    let data = json!({
        "DATABASE_URL": "postgres://admin:s3cretpassword@db.example.com:5432/app"
    });

    // password "s3cretpassword" = 14 chars → 20% keep = 2
    let redacted = redact_sensitive(&data, 20);

    assert_eq!(
        redacted["DATABASE_URL"],
        json!("postgres://admin:s3<redacted>rd@db.example.com:5432/app")
    );
}

#[test]
fn test_mask_partial_too_short_falls_back_to_full() {
    // Too short for a meaningful split at 20%.
    assert_eq!(mask_partial("ab", 20), "<redacted>");
    // keep*2 == len → full redact (nothing left for the middle).
    assert_eq!(mask_partial("abcd", 50), "<redacted>");
    assert_eq!(mask_partial("abcdefghij", 0), "<redacted>");
    // 5 chars at 50% → keep=2, middle of 1 → partial works.
    assert_eq!(mask_partial("short", 50), "sh<redacted>rt");
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

    assert!(
        name.ends_with(&today),
        "name should end with today's date: {name}"
    );
    assert!(
        name.contains("env"),
        "name should carry the filename: {name}"
    );
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

#[test]
fn test_query_run_clubs_names() {
    let data = parse_file("testdata/config.json", Some(Format::Json)).unwrap();
    let results = query::run(".clubs[] | .name", &data).unwrap();
    assert!(results.len() >= 3);
    assert_eq!(results[0], json!("Club de Regatas Vasco da Gama"));
}

#[test]
fn test_query_select_and_object() {
    let data = json!({
        "users": [
            {"id": 1, "name": "a", "active": true},
            {"id": 2, "name": "b", "active": false},
            {"id": 3, "name": "c", "active": true}
        ]
    });
    let results = query::run(".users[] | select(.active) | {id, name}", &data).unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], json!({"id": 1, "name": "a"}));
    assert_eq!(results[1], json!({"id": 3, "name": "c"}));
}

#[test]
fn test_legacy_path_still_works_alongside_query_module() {
    let data = json!({"club": {"name": "Vasco"}});
    let result = resolve_path(&data, "club.name").unwrap();
    assert_eq!(result, &json!("Vasco"));
    let q = query::run(".club.name", &data).unwrap();
    assert_eq!(q, vec![json!("Vasco")]);
}

/// Resolve the built confctl binary for CLI-level -q tests.
fn confctl_bin() -> Command {
    let bin = option_env!("CARGO_BIN_EXE_confctl").unwrap_or("confctl");
    let mut cmd = Command::new(bin);
    // Run from crate root so testdata/ paths resolve.
    cmd.current_dir(env!("CARGO_MANIFEST_DIR"));
    cmd.env_remove("CONFCTL_VAULT_PASSWORD");
    cmd
}

#[test]
fn cli_query_stream_one_name_per_line() {
    let output = confctl_bin()
        .args(["testdata/config.json", "-q", ".clubs[] | .name"])
        .output()
        .expect("run confctl");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), 4, "stdout={stdout}");
    assert_eq!(lines[0], "Club de Regatas Vasco da Gama");
    assert_eq!(lines[1], "Arsenal FC");
}

#[test]
fn cli_query_select_spain_compact_objects() {
    let output = confctl_bin()
        .args([
            "testdata/config.json",
            "-c",
            "-q",
            r#".clubs[] | select(.country == "Spain") | {name, country}"#,
        ])
        .output()
        .expect("run confctl");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<_> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 2, "stdout={stdout}");
    for line in &lines {
        let v: serde_json::Value = serde_json::from_str(line).expect(line);
        assert_eq!(v["country"], "Spain");
        assert!(v.get("name").is_some());
    }
}

#[test]
fn cli_query_and_path_together_fails() {
    let output = confctl_bin()
        .args([
            "testdata/config.json",
            "clubs.0.name",
            "-q",
            ".clubs[0].name",
        ])
        .output()
        .expect("run confctl");
    assert!(
        !output.status.success(),
        "expected failure when mixing path and -q"
    );
    let err = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        err.contains("either -q") || err.contains("not both"),
        "err={err}"
    );
}

#[test]
fn cli_legacy_path_unaffected() {
    let output = confctl_bin()
        .args(["testdata/config.json", "clubs.0.players.1.name"])
        .output()
        .expect("run confctl");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "Juninho Pernambucano"
    );
}

#[test]
fn cli_query_matches_legacy_path_leaf() {
    let path_out = confctl_bin()
        .args(["testdata/config.json", "clubs.2.titles.champions_league"])
        .output()
        .unwrap();
    let q_out = confctl_bin()
        .args([
            "testdata/config.json",
            "-q",
            ".clubs[2].titles.champions_league",
        ])
        .output()
        .unwrap();
    assert!(path_out.status.success());
    assert!(q_out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&path_out.stdout).trim(),
        String::from_utf8_lossy(&q_out.stdout).trim()
    );
    assert_eq!(String::from_utf8_lossy(&path_out.stdout).trim(), "15");
}

#[test]
fn cli_query_bad_syntax_exits_nonzero() {
    let output = confctl_bin()
        .args(["testdata/config.json", "-q", "if .x then 1"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}

#[test]
fn cli_query_missing_key_exits_nonzero() {
    let output = confctl_bin()
        .args(["testdata/config.json", "-q", ".does_not_exist"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}

#[test]
fn cli_query_empty_stream_succeeds_with_no_stdout() {
    let output = confctl_bin()
        .args(["testdata/config.json", "-q", ".clubs[] | select(false)"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
}

#[test]
fn cli_query_works_on_yaml_and_toml() {
    for file in ["testdata/config.yaml", "testdata/config.toml"] {
        let output = confctl_bin()
            .args([file, "-q", ".clubs | length"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{file}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "4");
    }
}

#[test]
fn cli_query_stdin_pipe() {
    let body = r#"[{"login":"alice"},{"login":"bob"}]"#;
    let mut child = confctl_bin()
        .args(["-q", ".[] | .login"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(body.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lines: Vec<_> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(lines, vec!["alice".to_string(), "bob".to_string()]);
}

#[test]
fn cli_query_redact_before_expression() {
    // Build a tiny env-like JSON with a secret; -r then -q must not leak secret.
    let dir = tempfile_dir();
    let path = dir.join("sec.json");
    std::fs::write(
        &path,
        r#"{"API_KEY":"sk-secret-value-12345","items":[{"n":1},{"n":2}]}"#,
    )
    .unwrap();
    let output = confctl_bin()
        .args([path.to_str().unwrap(), "-r", "-q", ".API_KEY"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let out = String::from_utf8_lossy(&output.stdout);
    assert!(out.contains("redacted"), "out={out}");
    assert!(!out.contains("sk-secret-value-12345"), "out={out}");
}

fn tempfile_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("confctl-query-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}
