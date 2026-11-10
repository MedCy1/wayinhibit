//! Command-line argument handling, against the real binary.

use std::process::Command;

#[test]
fn an_absurd_timeout_is_a_clean_usage_error_not_a_panic() {
    for timeout in [
        "99999999999999999h",
        "18446744073709551615s",
        "18446744073709551616s",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_wayinhibit"))
            .args(["--dry-run", "--quiet", "--timeout", timeout])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{timeout}: {stderr}");
        assert!(!stderr.contains("panicked"), "{timeout}: {stderr}");
        assert!(stderr.contains("invalid duration"), "{timeout}: {stderr}");
    }
}
