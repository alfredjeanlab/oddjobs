use super::parse_startup_error;

#[test]
fn parse_startup_error_with_blank_line_separator() {
    // The startup marker and ERROR line are separated by a blank line
    // for legibility when scanning daemon.log.
    let log = "\
--- ojd: starting (pid: 12345) ---

ERROR Failed to start daemon: address already in use
";
    let err = parse_startup_error(log).unwrap();
    assert_eq!(err, "address already in use");
}

#[test]
fn parse_startup_error_no_error() {
    let log = "\
--- ojd: starting (pid: 12345) ---

2026-01-01 INFO Starting user-level daemon
";
    assert!(parse_startup_error(log).is_none());
}

#[test]
fn parse_startup_error_multiple_startups_picks_last() {
    let log = "\
--- ojd: starting (pid: 100) ---

ERROR Failed to start daemon: first failure
--- ojd: starting (pid: 200) ---

ERROR Failed to start daemon: second failure
";
    let err = parse_startup_error(log).unwrap();
    assert_eq!(err, "second failure");
}

#[test]
fn parse_startup_error_no_marker() {
    let log = "some random log content\n";
    assert!(parse_startup_error(log).is_none());
}
