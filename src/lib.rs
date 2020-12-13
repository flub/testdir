//! Semi-persistent, scoped test directories
//!
//! This module provides a convenient way to have an empty directory for tests which can be
//! inspected after the test run in a predicatable location.  On subsequent test runs the
//! directory trees of previous runs will be cleaned up to keep the total number of
//! directories limited.
//!
//! # Quickstart
//!
//! ```no_run
//! mod tests {
//!     use std::path::PathBuf;
//!     use testdir::testdir;
//!
//!     #[test]
//!     fn test_write() {
//!         let dir: PathBuf = testdir!();
//!         let path = dir.join("hello.txt");
//!         std::fs::write(&path, "hi there").ok();
//!         assert!(path.exists());
//!     }
//!
//!     #[test]
//!     fn test_nonexisting() {
//!         let dir: PathBuf = testdir!();
//!         let path = dir.join("not-here.txt");
//!         assert!(!path.exists());
//!     }
//! }
//! # fn main() { }
//! ````
//!
//! If it does not yet exist this will create a directory called `rstest-of-$USER` in you
//! system's temporary directory.  Inside there you will find subdirectories named after
//! your crate name and with a number suffix which increases each time your run the tests.
//! There is also a `-current` suffix which symlinks to the most recent numbered directory.
//!
//! Inside the numbered directory you will find a directory structure resembling your
//! crate's modules structure.  For example if the above tests are in `lib.rs` of a crate
//! called `mycrate`, than on my UNIX system it looks like this:
//!
//! ```sh
//! $ tree /tmp/rstest-of-flub/
//! /tmp/rstest-of-flub/
//! +- mycrate-0/
//! |    +- mycrate/
//! |         +- tests/
//! |              +- test_nonexisting/
//! |              +- test_write/
//! |                   +- hello.txt
//! +- testdir-current -> /tmp/rstest-of-flub/mycrate-0
//! ```
//!
//! # Doc tests and Integration tests
//!
//! In order to re-use the same numbered test directories across [`testdir`] macro
//! invocations we need to have somewhere to keep global state.  This is currently stored in
//! a process-wide global.  However Cargo builds multiple test binaries in many situations,
//! typically a project will have the main crate test binary, one for each integration test
//! and one for each doctest.  The current implementation means each will get its own
//! numbered directory and the `-current` link will only point to the last one.  This could
//! be problematic if you have more test binaries than the number of numbered directories
//! kept by the [`testdir`] macro.  If you hit this limit a simple work-around is to limit
//! the test binaries you invoke if you need to inspect one of the tests.
//!
//! This should hopefully be fixed in the next version.

#![warn(missing_debug_implementations, clippy::all)]
// #![warn(missing_docs)]

use std::fs;
use std::num::NonZeroU8;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use once_cell::sync::Lazy;

#[cfg(unix)]
use std::os::unix::fs::symlink as symlink_dir;
#[cfg(windows)]
use std::os::windows::fs::symlink_dir;

/// The number of test directories retained by the [`testdir`] macro.
pub const KEEP_DEFAULT: Option<NonZeroU8> = NonZeroU8::new(8);

/// The global [`TestDir`] instance used by the [`testdir`] macro.
static TESTDIR: Lazy<RwLock<Option<TestDir>>> = Lazy::new(|| RwLock::new(None));

pub fn with_global_testdir<F, R>(base: &str, count: NonZeroU8, func: F) -> R
where
    F: FnOnce(&TestDir) -> R,
{
    with_global_testdir_with_tmpdir_provider(base, count, func, std::env::temp_dir)
}

// Testable version of with_global_testdir()
fn with_global_testdir_with_tmpdir_provider<F, R>(
    base: &str,
    count: NonZeroU8,
    func: F,
    provider: impl FnOnce() -> PathBuf,
) -> R
where
    F: FnOnce(&TestDir) -> R,
{
    let mut ro_guard = TESTDIR.read().unwrap();
    if ro_guard.is_none() {
        std::mem::drop(ro_guard);
        {
            let mut rw_guard = TESTDIR.write().unwrap();
            rw_guard.get_or_insert_with(|| {
                TestDir::new_user_with_tmpdir_provider(base, count, provider)
            });
        }
        ro_guard = TESTDIR.read().unwrap();
    }
    assert!(ro_guard.is_some());
    match *ro_guard {
        Some(ref test_dir) => func(test_dir),
        None => panic!("Global TESTDIR is None"),
    }
}

#[derive(Clone, Debug)]
pub struct TestDir {
    inner: NumberedDir,
}

impl TestDir {
    // New in $tmp
    pub fn new(root: &str, base: &str, count: NonZeroU8) -> Self {
        Self::new_with_tmpdir_provider(root, base, count, std::env::temp_dir)
    }

    // Testable version of Self::new()
    fn new_with_tmpdir_provider(
        root: &str,
        base: &str,
        count: NonZeroU8,
        provider: impl FnOnce() -> PathBuf,
    ) -> Self {
        let root_dir = provider().join(root);
        Self::new_abs(root_dir, base, count)
    }

    // New in $tmp/rstest-of-$user/
    pub fn new_user(base: &str, count: NonZeroU8) -> Self {
        Self::new_user_with_tmpdir_provider(base, count, std::env::temp_dir)
    }

    // Testable version of Self::new_user()
    fn new_user_with_tmpdir_provider(
        base: &str,
        count: NonZeroU8,
        provider: impl FnOnce() -> PathBuf,
    ) -> Self {
        let root = format!("rstest-of-{}", whoami::username());
        let root_dir = provider().join(root);
        Self::new_abs(root_dir, base, count)
    }

    // New with absolute path
    pub fn new_abs(root: impl AsRef<Path>, base: &str, count: NonZeroU8) -> Self {
        if !root.as_ref().exists() {
            fs::create_dir_all(root.as_ref()).expect("Failed to create root directory");
        }
        if !root.as_ref().is_dir() {
            panic!("Path for root is not a directory");
        }
        Self {
            inner: NumberedDir::new(root, base, count),
        }
    }

    pub fn path(&self) -> &Path {
        self.inner.path()
    }

    pub fn create_subdir(&self, rel_path: impl AsRef<Path>) -> PathBuf {
        self.inner.create_subdir(rel_path.as_ref())
    }
}

#[derive(Clone, Debug)]
pub struct NumberedDir {
    path: PathBuf,
}

impl NumberedDir {
    pub fn new(parent: impl AsRef<Path>, base: &str, count: NonZeroU8) -> Self {
        if base.contains('/') || base.contains('\\') {
            panic!("base must not contain path separators");
        }
        fs::create_dir_all(&parent).expect("Could not create parent");
        let next_count = match current_entry_count(&parent, base) {
            Some(current_count) => {
                remove_obsolete_dirs(&parent, base, current_count, u8::from(count) - 1);
                current_count.wrapping_add(1)
            }
            None => 0,
        };
        create_next_dir(&parent, base, next_count)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    // Creates numbered suffixes if dir already exists
    // Limitation: number of conflicting dirs
    // Safety: user can freely escape by using ../../somewhere/else
    // Performance: dozens of conflicts is fine, but don't do hundreds
    pub fn create_subdir(&self, rel_path: impl AsRef<Path>) -> PathBuf {
        let rel_path = rel_path.as_ref();
        assert!(rel_path.is_relative(), "not a relative path");
        let file_name = rel_path
            .file_name()
            .expect("subdir does not end in a file_name");

        if let Some(parent) = rel_path.parent() {
            let parent_path = self.path.join(parent);
            fs::create_dir_all(&parent_path).expect(&format!(
                "Failed to create subdir parent: {}",
                parent_path.display()
            ));
        }

        let mut full_path = self.path.join(&rel_path);
        for i in 0..u16::MAX {
            match fs::create_dir(&full_path) {
                Ok(_) => {
                    return full_path;
                }
                Err(_) => {
                    let mut new_file_name = file_name.to_os_string();
                    new_file_name.push(format!("-{}", i));
                    full_path.set_file_name(new_file_name);
                }
            }
        }
        panic!("subdir conflict: all filename alternatives exhausted");
    }
}

fn remove_obsolete_dirs(dir: impl AsRef<Path>, base: &str, current: u16, keep: u8) {
    let oldest = current.wrapping_sub(keep as u16).wrapping_add(1);

    for entry in NumberedEntryIter::new(&dir, base) {
        if current > oldest {
            if entry.number < oldest {
                let path = dir.as_ref().join(entry.name);
                fs::remove_dir_all(&path).expect(&format!("Failed to remove {}", path.display()));
            }
        } else {
            // We wrapped around u32::MAX

            // Avoid removing newly added entries by another process
            let min_remove = current + u8::MAX as u16;

            if min_remove < entry.number && entry.number < oldest {
                let path = dir.as_ref().join(entry.name);
                fs::remove_dir_all(&path).expect(&format!("Failed to remove {}", path.display()));
            }
        }
    }
}

fn create_next_dir(dir: impl AsRef<Path>, base: &str, mut next_count: u16) -> NumberedDir {
    let mut last_err = None;
    for _i in 0..16 {
        let name = format!("{}-{}", base, next_count);
        let path = dir.as_ref().join(name);
        match fs::create_dir(&path) {
            Ok(_) => {
                let current = dir.as_ref().join(format!("{}-current", base));
                if current.exists() {
                    fs::remove_file(&current).expect("Failed to remove previous current symlink");
                }
                // Could be racing other processes, should not fail
                symlink_dir(&path, &current).ok();
                return NumberedDir { path };
            }
            Err(err) => {
                next_count = next_count.wrapping_add(1);
                last_err = Some(err);
            }
        }
    }
    panic!(
        "Failed to create numbered dir, last error: {}",
        last_err.expect("no last error")
    );
}

fn current_entry_count(dir: impl AsRef<Path>, base: &str) -> Option<u16> {
    let mut max: Option<u16> = None;
    for entry in NumberedEntryIter::new(dir, base) {
        match max {
            Some(prev) => {
                if entry.number > prev {
                    max = Some(entry.number);
                }
            }
            None => {
                max = Some(entry.number);
            }
        }
    }
    max
}

#[derive(Clone, Debug)]
struct NumberedEntry {
    number: u16,
    name: String,
}

#[derive(Debug)]
struct NumberedEntryIter {
    prefix: String,
    readdir: fs::ReadDir,
}

impl NumberedEntryIter {
    fn new(dir: impl AsRef<Path>, base: &str) -> Self {
        Self {
            prefix: format!("{}-", base),
            readdir: dir
                .as_ref()
                .read_dir()
                .expect(&format!("Failed read_dir() on {}", dir.as_ref().display())),
        }
    }
}

impl Iterator for NumberedEntryIter {
    type Item = NumberedEntry;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let mut dirent = self.readdir.next()?;
            while dirent.is_err() {
                dirent = self.readdir.next()?;
            }
            let dirent = dirent.ok()?;
            let os_name = dirent.file_name();

            // We only work with valid UTF-8 entry names, so skip any names which are not.
            match os_name.to_str() {
                Some(name) => match name.strip_prefix(&self.prefix) {
                    Some(suffix) => match suffix.parse::<u16>() {
                        Ok(count) => {
                            return Some(NumberedEntry {
                                number: count,
                                name: name.to_string(),
                            });
                        }
                        Err(_) => continue,
                    },
                    None => continue,
                },
                None => continue,
            }
        }
    }
}

// consider moving this into the macro?
#[doc(hidden)]
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

// consider moving this into the macro?
#[doc(hidden)]
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

// testdir!() -> /tmp/rstest-of-me/crate/module/path/test_fn_name/
// testdir!(TestScope) -> /tmp/rstest-of-me/crate/module/path/test_fn_name/
// testdir!(ModuleScope) -> /tmp/rstest-of-me/crate/module/path/mod
// testdir!("some/path/name") -> /tmp/rstest-of-me/some/path/name/
// testdir!(Path::from("boo")) -> /tmp/rstest-of-me/boo/
#[macro_export]
macro_rules! testdir {
    () => {
        testdir!(TestScope)
    };
    ( TestScope ) => {{
        let pkg_name = ::std::env!("CARGO_PKG_NAME");
        let module_path = ::std::module_path!();
        let test_name = $crate::extract_test_name(&module_path);
        let subdir_path = ::std::path::Path::new(&module_path.replace("::", "/")).join(&test_name);
        // println!("macro pkg name: {}", pkg_name);
        // println!("macro module: {}", module_path);
        // println!("macro subdir: {}", subdir_path.display());
        // println!("macro test name: {}", test_name);
        $crate::with_global_testdir(pkg_name, $crate::KEEP_DEFAULT.unwrap(), move |tdir| {
            tdir.create_subdir(subdir_path)
        })
    }};
    ( ModuleScope ) => {{
        let pkg_name = ::std::env!("CARGO_PKG_NAME");
        let module_path = ::std::module_path!();
        let subdir_path = ::std::path::Path::new(&module_path.replace("::", "/")).join("mod");
        $crate::with_global_testdir(pkg_name, $crate::KEEP_DEFAULT.unwrap(), move |tdir| {
            tdir.create_subdir(subdir_path)
        })
    }};
    ( $e:expr ) => {{
        let pkg_name = ::std::env!("CARGO_PKG_NAME");

        $crate::with_global_testdir(pkg_name, $crate::KEEP_DEFAULT.unwrap(), move |tdir| {
            tdir.create_subdir($e)
        })
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_numbered_creation() {
        let parent = tempfile::tempdir().unwrap();
        let dir_0 = NumberedDir::new(parent.path(), "base", NonZeroU8::new(3).unwrap());
        assert_eq!(dir_0.path(), parent.path().join("base-0"));
        assert!(dir_0.path().is_dir());
    }

    #[test]
    fn test_numberd_creation_multiple() {
        let parent = tempfile::tempdir().unwrap();

        let dir_0 = NumberedDir::new(parent.path(), "base", NonZeroU8::new(3).unwrap());
        assert_eq!(dir_0.path(), parent.path().join("base-0"));
        assert!(dir_0.path().is_dir());

        let dir_1 = NumberedDir::new(parent.path(), "base", NonZeroU8::new(3).unwrap());
        assert_eq!(dir_1.path(), parent.path().join("base-1"));
        assert!(dir_0.path().is_dir());
        assert!(dir_1.path().is_dir());

        let dir_2 = NumberedDir::new(parent.path(), "base", NonZeroU8::new(3).unwrap());
        assert_eq!(dir_2.path(), parent.path().join("base-2"));
        assert!(dir_0.path().is_dir());
        assert!(dir_1.path().is_dir());
        assert!(dir_2.path().is_dir());

        let dir_3 = NumberedDir::new(parent.path(), "base", NonZeroU8::new(3).unwrap());
        assert_eq!(dir_3.path(), parent.path().join("base-3"));
        assert!(!dir_0.path().exists());
        assert!(dir_1.path().is_dir());
        assert!(dir_2.path().is_dir());
        assert!(dir_3.path().is_dir());

        let dir_4 = NumberedDir::new(parent.path(), "base", NonZeroU8::new(3).unwrap());
        assert_eq!(dir_4.path(), parent.path().join("base-4"));
        assert!(!dir_0.path().exists());
        assert!(!dir_1.path().exists());
        assert!(dir_2.path().is_dir());
        assert!(dir_3.path().is_dir());
        assert!(dir_4.path().is_dir());
    }

    #[test]
    fn test_numbered_creation_current() {
        let parent = tempfile::tempdir().unwrap();
        let dir_0 = NumberedDir::new(parent.path(), "base", NonZeroU8::new(3).unwrap());
        assert_eq!(dir_0.path(), parent.path().join("base-0"));
        assert!(dir_0.path().is_dir());

        let current = fs::read_link(parent.path().join("base-current")).unwrap();
        assert_eq!(dir_0.path(), current);

        let dir_1 = NumberedDir::new(parent.path(), "base", NonZeroU8::new(3).unwrap());
        assert_eq!(dir_1.path(), parent.path().join("base-1"));
        assert!(dir_0.path().is_dir());
        assert!(dir_1.path().is_dir());

        let current = fs::read_link(parent.path().join("base-current")).unwrap();
        assert_eq!(dir_1.path(), current);
    }

    #[test]
    fn test_numbered_subdir() {
        let parent = tempfile::tempdir().unwrap();
        let dir = NumberedDir::new(parent.path(), "base", NonZeroU8::new(3).unwrap());

        let sub = dir.create_subdir(Path::new("sub"));
        assert_eq!(sub, dir.path().join("sub"));
        assert!(sub.is_dir());

        let sub_0 = dir.create_subdir(Path::new("sub"));
        assert_eq!(sub_0, dir.path().join("sub-0"));
        assert!(sub_0.is_dir());
    }

    #[test]
    fn test_numbered_subdir_nested() {
        let parent = tempfile::tempdir().unwrap();
        let dir = NumberedDir::new(parent.path(), "base", NonZeroU8::new(3).unwrap());

        let sub = dir.create_subdir(Path::new("one/two"));
        assert_eq!(sub, dir.path().join("one/two"));
        assert!(dir.path().join("one").is_dir());
        assert!(dir.path().join("one").join("two").is_dir());
    }

    #[test]
    fn test_testdir_abs() {
        let parent = tempfile::tempdir().unwrap();
        let root_dir = parent.path().join("root");
        let tdir = TestDir::new_abs(&root_dir, "crate", NonZeroU8::new(3).unwrap());
        assert_eq!(tdir.path(), root_dir.join("crate-0"));
        assert!(tdir.path().is_dir());
    }

    #[test]
    fn test_testdir_new() {
        let parent = tempfile::tempdir().unwrap();
        let tdir =
            TestDir::new_with_tmpdir_provider("me", "crate", NonZeroU8::new(3).unwrap(), || {
                parent.path().to_path_buf()
            });
        assert_eq!(tdir.path(), parent.path().join("me").join("crate-0"));
        assert!(tdir.path().is_dir())
    }

    #[test]
    fn test_testdir_new_user() {
        let parent = tempfile::tempdir().unwrap();
        let tdir =
            TestDir::new_user_with_tmpdir_provider("crate", NonZeroU8::new(3).unwrap(), || {
                parent.path().to_path_buf()
            });
        let root = format!("rstest-of-{}", whoami::username());
        assert_eq!(tdir.path(), parent.path().join(root).join("crate-0"));
        assert!(tdir.path().is_dir())
    }

    #[test]
    fn test_with_global_testdir() {
        let parent = tempfile::tempdir().unwrap();
        let dir: PathBuf = with_global_testdir_with_tmpdir_provider(
            "crate",
            NonZeroU8::new(3).unwrap(),
            |tdir: &TestDir| tdir.create_subdir("test_with_global_testdir"),
            || parent.path().to_path_buf(),
        );
        let root = format!("rstest-of-{}", whoami::username());
        let expected_path = parent
            .path()
            .join(&root)
            .join("crate-0")
            .join("test_with_global_testdir");
        assert_eq!(dir, expected_path);
        assert!(dir.is_dir());

        let dir: PathBuf = with_global_testdir_with_tmpdir_provider(
            "crate-name-already-initialised",
            NonZeroU8::new(3).unwrap(),
            |tdir: &TestDir| tdir.create_subdir("test_with_global_testdir"),
            || parent.path().to_path_buf(),
        );
        let expected_path = parent
            .path()
            .join(&root)
            .join("crate-0")
            .join("test_with_global_testdir-0");
        assert_eq!(dir, expected_path);
        assert!(dir.is_dir());
    }
}
