//! A module with private functions to support the macros.
//!
//! This module is public, yet it contains some private functions.  This is to enable the
//! macros to function correctly without running into various dependency problems etc.  If
//! you do discover this module please do refrain from using it directly, there is no API
//! stability and this will violate semvers.

use std::ffi::OsStr;
use std::fs;
use std::path::Path;

// use heim::process::Pid;
use once_cell::sync::Lazy;
use sysinfo::{Pid, ProcessExt, SystemExt};

pub use cargo_metadata;

/// The filename in which we store the Cargo PID: `cargo-pid`.
const CARGO_PID_FILE_NAME: &str = "cargo-pid";

/// Whether we are a cargo sub-process.
// static CARGO_PID: Lazy<Option<Pid>> = Lazy::new(|| smol::block_on(async { cargo_pid().await }));
static CARGO_PID: Lazy<Option<Pid>> = Lazy::new(cargo_pid);

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
    let mut sys = sysinfo::System::new();
    let pid = sysinfo::get_current_pid().ok()?;
    let what = sysinfo::ProcessRefreshKind::new();
    sys.refresh_process_specifics(pid, what);
    let current = sys.process(pid)?;
    let ppid = current.parent()?;
    sys.refresh_process_specifics(ppid, what);
    let parent = sys.process(ppid)?;
    let parent_exe = parent.exe();
    let parent_file_name = parent_exe.file_name()?;
    if parent_file_name == OsStr::new("cargo") {
        Some(parent.pid())
    } else if parent_file_name == OsStr::new("rustdoc") {
        let ppid = parent.parent()?;
        sys.refresh_process_specifics(ppid, what);
        let parent = sys.process(ppid)?;
        let parent_exe = parent.exe();
        let parent_file_name = parent_exe.file_name()?;
        if parent_file_name == OsStr::new("cargo") {
            Some(parent.pid())
        } else {
            None
        }
    } else {
        None
    }
}

/// Determines if a [`NumberedDir`] was created by the same cargo parent process.
///
/// Commands like `cargo test` run various tests in sub-processes (unittests, doctests,
/// integration tests).  All of those subprocesses should re-use the same [`NumberedDir`].
/// This function figures out whether the given directory is the correct one or not.
///
/// [`NumberedDir`]: crate::NumberedDir
pub fn reuse_cargo(dir: &Path) -> bool {
    let file_name = dir.join(CARGO_PID_FILE_NAME);
    if let Ok(content) = fs::read_to_string(&file_name) {
        if let Ok(read_cargo_pid) = content.parse::<Pid>() {
            if let Some(cargo_pid) = *CARGO_PID {
                return read_cargo_pid == cargo_pid;
            }
        }
    }
    false
}

/// Creates a file storing the Cargo PID if not yet present.
///
/// # Panics
///
/// If the PID file could not be created or written.
pub fn create_cargo_pid_file(dir: &Path) {
    if let Some(cargo_pid) = *CARGO_PID {
        let file_name = dir.join(CARGO_PID_FILE_NAME);
        if !file_name.exists() {
            fs::write(&file_name, cargo_pid.to_string()).expect("Failed to write Cargo PID");
        }
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
pub fn extract_test_name_from_backtrace(module_path: &str) -> String {
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
    panic!("Cannot determine test name from backtrace");
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
