//! A module with private functions to support the macros.
//!
//! This module is public, yet it contains some private functions.  This is to enable the
//! macros to function correctly without running into various dependency problems etc.  If
//! you do discover this module please do refrain from using it directly, there is no API
//! stability and this will violate semvers.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{Context, Result, anyhow};
use fslock::LockFile;
use sysinfo::Pid;

use crate::{NumberedDir, NumberedDirBuilder};

/// The filename in which we store the Cargo PID: `cargo-pid`.
const CARGO_PID_FILE_NAME: &str = "cargo-pid";

/// The lockfile for creating the cargo PID file.
const CARGO_PID_LOCKFILE: &str = "cargo-pid.lock";

/// Whether we are a cargo sub-process.
static CARGO_PID: LazyLock<Option<Pid>> = LazyLock::new(cargo_pid);

#[cfg(target_family = "unix")]
const CARGO_NAME: &str = "cargo";

#[cfg(target_family = "unix")]
const NEXTEST_NAME: &str = "cargo-nextest";

#[cfg(target_family = "unix")]
const RUSTDOC_NAME: &str = "rustdoc";

#[cfg(target_family = "unix")]
const RUST_OUT_NAME: &str = "rust_out";

#[cfg(target_family = "windows")]
const CARGO_NAME: &str = "cargo.exe";

#[cfg(target_family = "windows")]
const NEXTEST_NAME: &str = "cargo-nextest.exe";

#[cfg(target_family = "windows")]
const RUSTDOC_NAME: &str = "rustdoc.exe";

#[cfg(target_family = "windows")]
const RUST_OUT_NAME: &str = "rust_out.exe";

/// Implementation of `crate::macros::init_testdir`.
pub fn init_testdir() -> NumberedDir {
    let parent = cargo_target_dir();
    let pkg_name = "testdir";
    let mut builder = NumberedDirBuilder::new(pkg_name.to_string());
    builder.set_parent(parent);
    builder.reusefn(reuse_cargo);
    let mut testdir = builder.create().expect("Failed to create testdir");
    let mut count = 0;
    while create_cargo_pid_file(testdir.path()).is_err() {
        count += 1;
        if count > 20 {
            break;
        }
        testdir = builder.create().expect("Failed to create testdir");
    }
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

    // Fast-path, just read the pidfile
    if let Some(read_cargo_pid) = fs::read_to_string(&file_name)
        .ok()
        .and_then(|content| content.parse::<Pid>().ok())
    {
        return Some(read_cargo_pid) == *CARGO_PID;
    }

    // Slow path, try and claim this directory for us. We are probably racing several
    // processes creating the next directory. Creating the pidfile uses a lockfile to make
    // sure only one process creates the pidfile.
    create_cargo_pid_file(dir).is_ok()
}

/// Creates a file storing the Cargo PID if not yet present.
///
/// Uses a lockfile to make sure only one process is writing the file at once. If the
/// pidfile was being written by another process at the same time and the PID matches it is
/// treated as a successful write.
///
/// # Returns
///
/// An error return indicates that the pid file was created by another process that was not
/// part of our testrun. So this numbered dir should not be used.
pub(crate) fn create_cargo_pid_file(dir: &Path) -> Result<()> {
    let cargo_pid = CARGO_PID
        .map(|pid| pid.to_string())
        .unwrap_or("failed to get cargo PID".to_string());

    // Lock the lockfile, unlocks when handle is dropped.
    let mut lockfile = LockFile::open(&dir.join(CARGO_PID_LOCKFILE))?;
    lockfile.lock()?;

    let file_name = dir.join(CARGO_PID_FILE_NAME);
    match File::create_new(&file_name) {
        Ok(_) => {
            fs::write(&file_name, cargo_pid).context("failed to write cargo-pid")?;
            Ok(())
        }
        Err(_) => {
            let read_pid = fs::read_to_string(&file_name)
                .context("failed to read cargo-pid")?
                .parse::<Pid>()?;
            if Some(read_pid) == *CARGO_PID {
                Ok(())
            } else {
                Err(anyhow::Error::msg("cargo PID does not match"))
            }
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
    let (ppid, parent_name) = parent_process(pid).ok()?;

    if parent_name == OsStr::new(CARGO_NAME) || parent_name == OsStr::new(NEXTEST_NAME) {
        // First parent is cargo or nextest directly for normal test runs.
        Some(ppid)
    } else if parent_name == OsStr::new(RUST_OUT_NAME) {
        // Edition 2024 can have additional binary in between the test process and the
        // doctest parent when it merges doc tests.
        let (doctest_pid, doctest_name) = parent_process(ppid).ok()?;
        if doctest_name == OsStr::new(RUSTDOC_NAME) {
            // The parent of this should be cargo.
            let (cargo_pid, cargo_name) = parent_process(doctest_pid).ok()?;
            // Nextest does not run doc tests, only look for cargo itself.
            if cargo_name == OsStr::new(CARGO_NAME) {
                Some(cargo_pid)
            } else {
                None
            }
        } else {
            None
        }
    } else if parent_name == OsStr::new(RUSTDOC_NAME) {
        // Before edition 2024 or if the doc test could not be merged we have doctest as
        // parent directly.
        let (cargo_pid, cargo_name) = parent_process(ppid).ok()?;
        // Nextest does not run doc tests, only look for cargo itself.
        if cargo_name == OsStr::new(CARGO_NAME) {
            Some(cargo_pid)
        } else {
            None
        }
    } else {
        None
    }
}

/// Returns the pid and name of the parent process of `pid`.
fn parent_process(pid: Pid) -> Result<(Pid, OsString)> {
    let mut sys = sysinfo::System::new();
    let what = sysinfo::ProcessRefreshKind::nothing().with_exe(sysinfo::UpdateKind::Always);

    // Find the parent pid
    sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::Some(&[pid]), false, what);
    let process = sys.process(pid).ok_or(anyhow!("failed process fetch"))?;
    let ppid = process.parent().ok_or(anyhow!("no parent process"))?;

    // Find the parent name
    sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::Some(&[ppid]), false, what);
    let parent = sys.process(ppid).ok_or(anyhow!("failed parent fetch"))?;
    let parent_name = parent
        .exe()
        .and_then(|exe| exe.file_name())
        .unwrap_or_else(|| parent.name());
    Ok((ppid, parent_name.to_os_string()))
}

/// Extracts the name of the currently executing test.
pub fn extract_test_name(module_path: &str) -> String {
    let mut name = std::thread::current()
        .name()
        .unwrap_or_default()
        .to_string();
    if name == "main" {
        name = extract_test_name_from_backtrace(module_path);
    }
    if let Some((_head, tail)) = name.split_once("::") {
        // The test name usually starts with the module name, so skip that.
        name = tail.to_string();
    }
    // When using rstest the test name still contains multiple module paths entries, they
    // are named "test_name::case_name". So we turn the name into several path entries:
    // "test_name/case_name".
    name.replace("::", ::std::path::MAIN_SEPARATOR_STR)
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
