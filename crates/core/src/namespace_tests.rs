use super::*;
use std::path::PathBuf;

// --- namespace_to_option tests ---

#[test]
fn namespace_to_option_empty_returns_none() {
    assert_eq!(namespace_to_option(""), None);
}

#[test]
fn namespace_to_option_non_empty_returns_some() {
    assert_eq!(namespace_to_option("proj"), Some("proj"));
}

// --- Namespace newtype tests ---

#[test]
fn namespace_new_and_deref() {
    let ns = Namespace::new("myproj");
    assert_eq!(&*ns, "myproj");
    assert!(!ns.is_empty());
}

#[test]
fn namespace_default_is_empty() {
    let ns = Namespace::default();
    assert!(ns.is_empty());
    assert_eq!(&*ns, "");
}

#[test]
fn namespace_to_option_method_empty() {
    let ns = Namespace::default();
    assert_eq!(ns.to_option(), None);
}

#[test]
fn namespace_to_option_method_non_empty() {
    let ns = Namespace::new("proj");
    assert_eq!(ns.to_option(), Some("proj"));
}

#[test]
fn namespace_display() {
    let ns = Namespace::new("hello");
    assert_eq!(format!("{ns}"), "hello");
}

#[test]
fn namespace_from_string() {
    let ns: Namespace = String::from("test").into();
    assert_eq!(&*ns, "test");
}

#[test]
fn namespace_from_str() {
    let ns: Namespace = "test".into();
    assert_eq!(&*ns, "test");
}

#[test]
fn namespace_into_inner() {
    let ns = Namespace::new("proj");
    let s: String = ns.into_inner();
    assert_eq!(s, "proj");
}

#[test]
fn namespace_serde_roundtrip() {
    let ns = Namespace::new("myproject");
    let json = serde_json::to_string(&ns).unwrap();
    assert_eq!(json, "\"myproject\"");
    let deserialized: Namespace = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, ns);
}

#[test]
fn namespace_deref_coercion_with_scoped_name() {
    let ns = Namespace::new("proj");
    // Namespace deref-coerces to &str for functions taking &str
    assert_eq!(scoped_name(&ns, "queue"), "proj/queue");
}

// --- scoped_name / split_scoped_name tests ---

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
