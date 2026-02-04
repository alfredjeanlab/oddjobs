use super::*;
use std::path::PathBuf;

#[test]
fn scoped_name_with_namespace() {
    assert_eq!(scoped_name("proj", "queue1"), "proj/queue1");
}

#[test]
fn scoped_name_empty_namespace() {
    assert_eq!(scoped_name("", "queue1"), "queue1");
}

#[test]
fn split_scoped_name_with_namespace() {
    assert_eq!(split_scoped_name("proj/queue1"), ("proj", "queue1"));
}

#[test]
fn split_scoped_name_bare_name() {
    assert_eq!(split_scoped_name("queue1"), ("", "queue1"));
}

#[test]
fn split_scoped_name_roundtrip() {
    let scoped = scoped_name("ns", "name");
    let (ns, name) = split_scoped_name(&scoped);
    assert_eq!(ns, "ns");
    assert_eq!(name, "name");
}

#[test]
fn split_scoped_name_empty_roundtrip() {
    let scoped = scoped_name("", "bare");
    let (ns, name) = split_scoped_name(&scoped);
    assert_eq!(ns, "");
    assert_eq!(name, "bare");
}

#[test]
fn resolve_from_config_file() {
    let dir = tempfile::tempdir().unwrap();
    let oj_dir = dir.path().join(".oj");
    std::fs::create_dir_all(&oj_dir).unwrap();
    std::fs::write(
        oj_dir.join("config.toml"),
        "[project]\nname = \"myproject\"\n",
    )
    .unwrap();
    assert_eq!(resolve_namespace(dir.path()), "myproject");
}

#[test]
fn resolve_fallback_to_dirname() {
    let dir = tempfile::tempdir().unwrap();
    let result = resolve_namespace(dir.path());
    let expected = dir
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(result, expected);
}

#[test]
fn resolve_fallback_root_path() {
    assert_eq!(resolve_namespace(&PathBuf::from("/")), "default");
}

#[test]
fn resolve_ignores_malformed_config() {
    let dir = tempfile::tempdir().unwrap();
    let oj_dir = dir.path().join(".oj");
    std::fs::create_dir_all(&oj_dir).unwrap();
    std::fs::write(oj_dir.join("config.toml"), "not valid toml {{{\n").unwrap();
    // Should fall back to dirname
    let result = resolve_namespace(dir.path());
    let expected = dir
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(result, expected);
}

#[test]
fn resolve_ignores_config_without_project_name() {
    let dir = tempfile::tempdir().unwrap();
    let oj_dir = dir.path().join(".oj");
    std::fs::create_dir_all(&oj_dir).unwrap();
    std::fs::write(oj_dir.join("config.toml"), "[other]\nkey = \"val\"\n").unwrap();
    let result = resolve_namespace(dir.path());
    let expected = dir
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(result, expected);
}
