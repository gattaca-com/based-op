use alloy_primitives::hex;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_cli_help() {
    let output = Command::new("cargo")
        .args(["run", "--bin", "based-portal", "--", "--help"])
        .output()
        .expect("Failed to run based-portal");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: based-portal"));
    assert!(stdout.contains("--port"));
    assert!(stdout.contains("--config-dir"));
}

#[test]
fn test_cli_default_args() {
    let output = Command::new("cargo")
        .args(["run", "--bin", "based-portal", "--", "--help"])
        .output()
        .expect("Failed to run based-portal");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Check for default values in the help output
    assert!(stdout.contains("8080"));
    assert!(stdout.contains("/config"));
    assert!(stdout.contains("bop-portal.log"));
    assert!(stdout.contains("http://0.0.0.0:8545"));
    assert!(stdout.contains("http://0.0.0.0:8551"));
}

#[test]
fn test_cli_invalid_args() {
    let output = Command::new("cargo")
        .args(["run", "--bin", "based-portal", "--", "--invalid-flag"])
        .output()
        .expect("Failed to run based-portal");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error:"));
}

#[test]
fn test_jwt_file_creation() {
    let dir = tempdir().unwrap();
    let jwt_path = dir.path().join("jwt");

    // Create a test JWT file
    let jwt_bytes = [0x42u8; 32];
    let hex_jwt = hex::encode(jwt_bytes);
    fs::write(&jwt_path, &hex_jwt).unwrap();

    // Test that the file exists and has correct content
    let content = fs::read_to_string(&jwt_path).unwrap();
    assert_eq!(content, hex_jwt);
}
