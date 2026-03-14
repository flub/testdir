//! A module with private functions to support the macros.
//!
//! This module is public, yet it contains some private functions.  This is to enable the
//! macros to function correctly without running into various dependency problems etc.  If
//! you do discover this module please do refrain from using it directly, there is no API
//! stability and this will violate semvers.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use sysinfo::Pid;

use crate::{NumberedDir, NumberedDirBuilder};

/// The filename in which we store the Cargo PID: `cargo-pid`.
const CARGO_PID_FILE_NAME: &str = "cargo-pid";

/// Whether we are a cargo sub-process.
static CARGO_PID: LazyLock<Option<Pid>> = LazyLock::new(cargo_pid);

#[cfg(target_family = "unix")]
const CARGO_NAME: &str = "cargo";

#[cfg(target_family = "unix")]
const NEXTEST_NAME: &str = "cargo-nextest";

#[cfg(target_family = "windows")]
const CARGO_NAME: &str = "cargo.exe";

#[cfg(target_family = "windows")]
const NEXTEST_NAME: &str = "cargo-nextest.exe";

/// Implementation of `crate::macros::init_testdir`.
pub fn init_testdir() -> NumberedDir {
    let parent = cargo_target_dir();
    let pkg_name = "testdir";
    let mut builder = NumberedDirBuilder::new(pkg_name.to_string());
    builder.set_parent(parent);
    builder.reusefn(reuse_cargo);
    let testdir = builder.create().expect("Failed to create testdir");
    create_cargo_pid_file(testdir.path());
    testdir
}

/// Returns the cargo target directory or a best guess.
///
/// This aims to return the cargo target directory. Though in some environments like
/// cargo-dinghy cargo_metadata is not available. In those cases the directory of the test
/// executable is used, which usually is somewhere in the target directory.
fn cargo_target_dir() -> PathBuf {
    match cargo_metadata::MetadataCommand::new().exec() {
        Ok(metadata) => metadata.target_directory.into(),
        Err(_) => {
            // In some environments cargo-metadata is not available,
            // e.g. cargo-dinghy.  Use the directory of test executable.
            let current_exe = ::std::env::current_exe().expect("no current exe");
            current_exe
                .parent()
                .expect("no parent dir for current exe")
                .into()
        }
    }
}

/// Determines if a [`NumberedDir`] was created by the same cargo parent process.
///
/// Commands like `cargo test` run various tests in sub-processes (unittests, doctests,
/// integration tests).  All of those subprocesses should re-use the same [`NumberedDir`].
/// This function figures out whether the given directory is the correct one or not.
///
/// [`NumberedDir`]: crate::NumberedDir
pub(crate) fn reuse_cargo(dir: &Path) -> bool {
    let file_name = dir.join(CARGO_PID_FILE_NAME);
    let start = Instant::now();
    while start.elapsed() <= Duration::from_millis(500) {
        if let Some(read_cargo_pid) = fs::read_to_string(&file_name)
            .ok()
            .and_then(|content| content.parse::<Pid>().ok())
        {
            return Some(read_cargo_pid) == *CARGO_PID;
        } else {
            // Wen we encounter a directory that has no pidfile we assume some other process
            // just created the directory and is about to write the pdifile. So we wait a
            // little in the hope the pidfile appears.
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    // Give up, we'll create a new directory ourselves.
    false
}

/// Creates a file storing the Cargo PID if not yet present.
///
/// # Panics
///
/// If the PID file could not be created or written.
pub(crate) fn create_cargo_pid_file(dir: &Path) {
    if let Some(cargo_pid) = *CARGO_PID {
        let file_name = dir.join(CARGO_PID_FILE_NAME);
        if !file_name.exists() {
            fs::write(&file_name, cargo_pid.to_string()).expect("Failed to write Cargo PID");
        }
    }
}

/// Returns the process ID of our parent Cargo process.
///
/// If our parent process is not Cargo, `None` is returned.
//
// ```
// use testdir::testdir;
//
// let dir = testdir!("ham");
// println!("dir {:}", dir.display());
// let pidfile = dir.join("../cargo-pid");
// assert!(pidfile.is_file());
// ```
fn cargo_pid() -> Option<Pid> {
    let pid = sysinfo::get_current_pid().ok()?;

    let mut sys = sysinfo::System::new();
    let what = sysinfo::ProcessRefreshKind::nothing().with_exe(sysinfo::UpdateKind::OnlyIfNotSet);
    sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::Some(&[pid]), false, what);
    let current = sys.process(pid)?;
    let ppid = current.parent()?;
    sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::Some(&[ppid]), false, what);
    let parent = sys.process(ppid)?;
    let parent_name = parent
        .exe()
        .and_then(|exe| exe.file_name())
        .unwrap_or_else(|| parent.name());
    if parent_name == OsStr::new(CARGO_NAME) || parent_name == OsStr::new(NEXTEST_NAME) {
        Some(parent.pid())
    } else if parent_name == OsStr::new("rustdoc") {
        let ppid = parent.parent()?;
        sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::Some(&[ppid]), false, what);
        let parent = sys.process(ppid)?;
        let parent_name = parent
            .exe()
            .and_then(|exe| exe.file_name())
            .unwrap_or_else(|| parent.name());
        if parent_name == OsStr::new("cargo") {
            Some(parent.pid())
        } else {
            None
        }
    } else {
        None
    }
}

/// Extracts the name of the currently executing test.
pub fn extract_test_name(module_path: &str) -> String {
    let mut name = std::thread::current()
        .name()
        .expect("Test thread has no name, can not find test name")
        .to_string();
    if name == "main" {
        name = extract_test_name_from_backtrace(module_path);
    }
    if let Some(tail) = name.rsplit("::").next() {
        name = tail.to_string();
    }
    name
}

/// Extracts the name of the currently executing tests using [`backtrace`].
fn extract_test_name_from_backtrace(module_path: &str) -> String {
    for symbol in backtrace::Backtrace::new()
        .frames()
        .iter()
        .rev()
        .flat_map(|x| x.symbols())
        .filter_map(|x| x.name())
        .map(|x| x.to_string())
    {
        if let Some(symbol) = symbol.strip_prefix(module_path) {
            if let Some(symbol) = symbol.strip_suffix("::{{closure}}") {
                return symbol.to_string();
            } else {
                return symbol.to_string();
            }
        }
    }

    // We know that on windows doc tests fall through as the module_path is something like
    // "rust_out" which is not very useful.  We'll have to just use something.
    String::from("unknown_test")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cargo_pid() {
        let val = cargo_pid();
        assert!(val.is_some());
    }
}
