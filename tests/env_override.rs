use std::path::PathBuf;

use testdir::testdir;

#[test]
fn test_rust_testdir_env() {
    let custom_dir = std::env::temp_dir().join("testdir-env-test");
    // Clean up from any previous run.
    let _ = std::fs::remove_dir_all(&custom_dir);

    // Run a subprocess with RUST_TESTDIR set so we get a fresh OnceLock.
    let exe = std::env::current_exe().unwrap();
    let output = std::process::Command::new(exe)
        .arg("--exact")
        .arg("test_rust_testdir_env_inner")
        .env("RUST_TESTDIR", &custom_dir)
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "subprocess failed: {stderr}");

    // The directory itself should be the testdir root — no testdir-N inside it.
    assert!(custom_dir.is_dir(), "RUST_TESTDIR was not created");

    // There should be no testdir-N intermediate directories.
    let has_numbered = std::fs::read_dir(&custom_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| e.file_name().to_string_lossy().starts_with("testdir-"));
    assert!(
        !has_numbered,
        "RUST_TESTDIR should not contain testdir-N subdirs"
    );

    // The file written by the inner test should exist at the expected path.
    let marker = custom_dir.join("env_override/test_rust_testdir_env_inner/marker.txt");
    assert!(marker.is_file(), "marker file not found at {}", marker.display());
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "hello from env override");

    // Cleanup
    let _ = std::fs::remove_dir_all(&custom_dir);
}

#[test]
fn test_rust_testdir_env_inner() {
    // Called from test_rust_testdir_env with RUST_TESTDIR set.
    // When called directly (without the env var), it just runs as a normal test.
    if let Some(expected_parent) = std::env::var_os("RUST_TESTDIR") {
        let dir: PathBuf = testdir!();
        let expected = PathBuf::from(expected_parent);
        assert!(
            dir.starts_with(&expected),
            "testdir {} is not under {}",
            dir.display(),
            expected.display()
        );

        let marker = dir.join("marker.txt");
        std::fs::write(&marker, "hello from env override").unwrap();
    }
}
