use std::path::PathBuf;
use std::process::Command;

fn binary_path() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_cosmic_ssh_askpass") {
        return PathBuf::from(path);
    }
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("..");
    path.push("target");
    path.push("debug");
    path.push("cosmic-ssh-askpass");
    path
}

fn askpass_with_read(read_file: &str) -> Command {
    let mut cmd = Command::new(binary_path());
    cmd.env_remove("OO7_PASSPHRASE_READ_FILE");
    cmd.env_remove("OO7_PASSPHRASE_WRITE_FILE");
    cmd.env("OO7_PASSPHRASE_READ_FILE", read_file);
    cmd
}

#[test]
fn test_read_file_prints_passphrase() {
    let dir = std::env::temp_dir();
    let path = dir.join("askpass-test-read");
    std::fs::write(&path, "cached_pass").unwrap();

    let output = askpass_with_read(path.to_str().unwrap())
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "cached_pass");
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_read_file_preserves_newlines() {
    let dir = std::env::temp_dir();
    let path = dir.join("askpass-test-newline");
    std::fs::write(&path, "pass\nword").unwrap();

    let output = askpass_with_read(path.to_str().unwrap())
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "pass\nword");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_read_file_takes_priority_over_cli_args() {
    let dir = std::env::temp_dir();
    let path = dir.join("askpass-test-priority");
    std::fs::write(&path, "from_cache").unwrap();

    let mut cmd = askpass_with_read(path.to_str().unwrap());
    cmd.arg("Custom prompt");
    let output = cmd.output().unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "from_cache",
        "read file should be used even when args are present"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_multiple_arguments_ignored_when_read_file() {
    let dir = std::env::temp_dir();
    let path = dir.join("askpass-test-multi");
    std::fs::write(&path, "cached").unwrap();

    let mut cmd = askpass_with_read(path.to_str().unwrap());
    let _ = cmd.arg("arg1").arg("arg2").arg("arg3");
    let output = cmd.output().unwrap();

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "cached");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_existing_read_file_with_write_env_unset() {
    let dir = std::env::temp_dir();
    let path = dir.join("askpass-test-write");
    std::fs::write(&path, "pas123").unwrap();

    let mut cmd = askpass_with_read(path.to_str().unwrap());
    cmd.env_remove("OO7_PASSPHRASE_WRITE_FILE");
    let output = cmd.output().unwrap();

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "pas123");
    let _ = std::fs::remove_file(&path);
}
