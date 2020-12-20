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

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::num::NonZeroU8;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use heim::process::Pid;
use once_cell::sync::{Lazy, OnceCell};

#[cfg(unix)]
use std::os::unix::fs::symlink as symlink_dir;
#[cfg(windows)]
use std::os::windows::fs::symlink_dir;

/// Default to build the `root` for [`NumberedDirBuilder`] from: `testdir`.
pub const ROOT_DEFAULT: &'static str = "testdir";

/// The default number of test directories retained by [`NumberedDirbuilder`]: `8`.
pub const KEEP_DEFAULT: Option<NonZeroU8> = NonZeroU8::new(8);

/// The filename in which the [`testdir`] macro stores the Cargo PID.
pub const CARGO_PID_FILE_NAME: &'static str = "cargo-pid";

/// The global [`NumberedDir`] instance used by the [`testdir`] macro.
static TESTDIR: OnceCell<NumberedDir> = OnceCell::new();

/// The global [`NumberedDirBuilder`] instance used by the [`testdir`] macro.
///
/// The [`testdir`] macro assumes it can initialise this, if you initialise it before
/// calling this macro you can modify the [`NumberedDirBuilder`] used by the macro.
pub static TESTDIR_BUILDER: OnceCell<NumberedDirBuilder> = OnceCell::new();

/// Whether we are a cargo sub-process.
static CARGO_PID: Lazy<Option<Pid>> = Lazy::new(|| smol::block_on(async { cargo_pid().await }));

/// Executes a function passing the global [`NumberedDir`] instance.
///
/// This is used by the [`testdir`] macrot to create subdirectories inside one global
/// [`NumberedDir`] instance for each test using [`NumberedDir::subdir`].  You may use this
/// for similar purposes.
pub fn with_global_testdir<F, R>(builder: &NumberedDirBuilder, func: F) -> R
where
    F: FnOnce(&NumberedDir) -> R,
{
    let test_dir = TESTDIR.get_or_init(|| builder.create());
    func(test_dir)
}

/// Builder to create a [`NumberedDir`].
///
/// While you can use [`NumberedDir::new`] directly this provides functionality to specific
/// ways of constructing and re-using the [`NumberedDir`].
///
/// Primarily this builder adds the concept of a **root**, a directory in which to create
/// the [`NumberedDir`].  The concept of the **base** is the same as for [`NumberedDir`] and
/// is the prefix of the name of the [`NumberedDir`], thus a prefix of `myprefix` would
/// create directories numbered `myprefix-0`, `myprefix-1` etc.  Likewise the **count** is
/// also the same concept as for [`NumberedDir`] and specifies the maximum number of
/// numbered directories, older directories will be cleaned up.
///
/// # Configuring the builder
///
/// The basic constructor uses a *root* of `testdir-of-$USER` placed in the system's default
/// temporary director location as per [`std::env::temp_dir`].  To customise the root you
/// can use [`NumberdDirBuilder::root`] or [`NumberedDirBuilder::user_root].  The temporary
/// directory provider can also be changed using [`NumberedDirBuilder::tmpdir_provider`].
///
/// If you simply want an absolute path as parent directory for the numbered directory use
/// the [`NumberedDirBuilder::abs_root`] function.
///
/// # Creating the [`NumberedDir`]
///
/// The [`NumberedDirBuilder::create`] method will create a new [`NumberedDir`].  In some
/// situations you may want to re-use a previous numbered directory which you can do using
/// [`NumberedDirBuilder::create_or_reuse].
///
/// This is useful for example when running tests using `cargo test` and you want to use the
/// same numbered directory for the unit, integration and doc tests even though they all run
/// in different processes.  The [`NumberdedDirBuilder::create_or_reuse_cargo`] method does
/// this by storing the process ID of the `cargo test` directory in the numbered directory
/// and comparing that to the parent process ID of the current process.
#[derive(Clone)]
pub struct NumberedDirBuilder {
    // The current absolute path of the parent directory.  The last component is the current
    // root.  This is the parent directory in which we should create the NumberedDir.
    parent: PathBuf,
    // The base of the numbered dir, its name without the number suffix.
    base: String,
    // The number of numbered dirs to keep around **after** the new directory is created.
    count: NonZeroU8,
    // Function to determine whether to re-use a numbered dir.
    reuse_fn: Option<Arc<Box<dyn Fn(&Path) -> bool + Send + Sync>>>,
}

impl fmt::Debug for NumberedDirBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NumberedDirBuilder")
            .field("parent", &self.parent)
            .field("base", &self.base)
            .field("count", &self.count)
            .field("reusefn", &"<Fn(&Path) -> bool>")
            .finish()
    }
}

impl NumberedDirBuilder {
    /// Create a new builder for [`NumberedDir`].
    ///
    /// By default the *root* will be set to `testdir-of-$USER`. (using [`ROOT_DEFAULT`])
    /// and the count will be set to `8` ([`KEEP_DEFAULT`]).
    pub fn new(base: String) -> Self {
        if base.contains('/') || base.contains('\\') {
            panic!("base must not contain path separators");
        }
        let root = format!("{}-of-{}", ROOT_DEFAULT, whoami::username());
        Self {
            parent: std::env::temp_dir().join(root),
            base,
            count: KEEP_DEFAULT.unwrap(),
            reuse_fn: None,
        }
    }

    /// Resets the *base*-name of the [`NumberedDir`].
    pub fn base(&mut self, base: String) -> &mut Self {
        self.base = base;
        self
    }

    /// Sets a *root* in the system's temporary directory location.
    ///
    /// The [`NumberedDir`]'s parent will be the `root` subdirectory of the system's
    /// default temporary directory location.
    pub fn root(&mut self, root: impl Into<String>) -> &mut Self {
        self.parent.set_file_name(root.into());
        self
    }

    /// Sets a *root* with the username affixed.
    ///
    /// Like [`NumberedDirBuilder::root`] this sets a subdirectory of the system's default
    /// temporary directory location as the parent direcotry for the [`NumberedDir`].
    /// However it suffixes the username to the given `prefix` to use as *root*.
    pub fn user_root(&mut self, prefix: &str) -> &mut Self {
        let root = format!("{}{}", prefix, whoami::username());
        self.parent.set_file_name(root);
        self
    }

    /// Uses a different temporary direcotry to place the *root* into.
    ///
    /// By default [`std::env::temp_dir`] is used to get the system's temporary directory
    /// location to place the *root* into.  This allows you to provide an alternate function
    /// which will be called to get the location of the directory where *root* will be
    /// placed.  You provider should probably return an absolute path but this is not
    /// enforced.
    pub fn tmpdir_provider(&mut self, provider: impl FnOnce() -> PathBuf) -> &mut Self {
        let default_root = OsString::from_str(ROOT_DEFAULT).unwrap();
        let root = self.parent.file_name().unwrap_or(&default_root);
        self.parent = provider().join(root);
        self
    }

    /// Sets the parent directory for the [`NumberedDir`].
    ///
    /// This does not follow the *root* concept anymore, instead it directly sets the full
    /// path for the parent directory in which the [`NumberedDir`] will be created.  You
    /// probably want this to be an absolute path but this is not enforced.
    ///
    /// Be aware that it is a requirement that the last component of the parent directory is
    /// valid UTF-8.
    pub fn set_parent(&mut self, path: PathBuf) -> &mut Self {
        if path.file_name().and_then(|name| name.to_str()).is_none() {
            panic!("Last component of parent is not UTF-8");
        }
        self.parent = path;
        self
    }

    /// Sets the total number of [`NumberedDir`] directories to keep.
    ///
    /// If creating the new [`NumberedDir`] would exceed this number, older directories will
    /// be removed.
    pub fn count(&mut self, count: NonZeroU8) -> &mut Self {
        self.count = count;
        self
    }

    /// Enables [`NumberedDir`] re-use if `f` returns `true`.
    ///
    /// The provided function will be called with each existing numbered directory and if it
    /// returns `true` this directory will be re-used instead of a new one being created.
    pub fn reusefn<F>(&mut self, f: F) -> &mut Self
    where
        F: Fn(&Path) -> bool + Send + Sync + 'static,
    {
        self.reuse_fn = Some(Arc::new(Box::new(f)));
        self
    }

    /// Disables any previous call to [`NumberedDirBuilder::reusefn`].
    pub fn disable_reuse(&mut self) -> &mut Self {
        self.reuse_fn = None;
        self
    }

    /// Creates a new [`NumberedDir`] as configured.
    pub fn create(&self) -> NumberedDir {
        if !self.parent.exists() {
            fs::create_dir_all(&self.parent).expect("Failed to create root directory");
        }
        if !self.parent.is_dir() {
            panic!("Path for root is not a directory");
        }
        if let Some(ref reuse_fn) = self.reuse_fn {
            for entry in NumberedEntryIter::new(&self.parent, &self.base) {
                let path = self.parent.join(&entry.name);
                if reuse_fn(&path) {
                    return NumberedDir {
                        path: path.to_path_buf(),
                    };
                }
            }
        }
        NumberedDir::new(&self.parent, &self.base, self.count)
    }
}

/// Determines if a [`NumberedDir`] was created by the same cargo parent process.
///
/// Commands like `cargo test` run various tests in sub-processes (unittests, doctests,
/// integration tests).  All of those subprocesses should re-use the same [`NumberedDir`].
/// This function figures out whether the given directory is the correct one or not.
#[doc(hidden)]
pub fn __private_reuse_cargo(dir: &Path) -> bool {
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
#[doc(hidden)]
pub fn __private_create_cargo_pid_file(dir: &Path) {
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
async fn cargo_pid() -> Option<Pid> {
    let current = heim::process::current().await.ok()?;
    let parent = current.parent().await.ok()?;
    let parent_exe = parent.exe().await.ok()?;
    let parent_file_name = parent_exe.file_name()?;
    if parent_file_name == OsStr::new("cargo") {
        None
    } else {
        Some(parent.pid())
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

/// Remove obsolete numbered directories.
///
/// The [`NumberedDir`] is identified by the parent directory `dir` and its base name
/// `base`.  It will retain a maximum number of `keep` directories starting from `current`.
/// By setting `keep` to `0` it will remove all directories.
///
/// Any directories with higher numbers than `current` will be left alone as they are
/// assumed to be created by concurrent processes creating the same numbered directories.
fn remove_obsolete_dirs(dir: impl AsRef<Path>, base: &str, current: u16, keep: u8) {
    let oldest_to_keep = current.wrapping_sub(keep as u16).wrapping_add(1);
    let oldest_to_delete = current.wrapping_add(u16::MAX / 2);
    assert!(oldest_to_keep != oldest_to_delete);

    for entry in NumberedEntryIter::new(&dir, base) {
        if (oldest_to_keep > oldest_to_delete
            && (entry.number < oldest_to_keep && entry.number >= oldest_to_delete))
            || (oldest_to_keep < oldest_to_delete
                && (entry.number < oldest_to_keep || entry.number >= oldest_to_delete))
        {
            let path = dir.as_ref().join(entry.name);
            fs::remove_dir_all(&path).expect(&format!("Failed to remove {}", path.display()));
        }
    }
}

/// Attempt to create the next numbered directory.
///
/// The directory will be placed in `dir` and its name composed of the `base` and
/// `next_count`.  If this directory can not be created it is assumed another process
/// created it already and the count is increased and tried again.  This is repeated maximum
/// 16 times after which this gives up.
///
/// Once the directory is created the `-current` symlink is also created.
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
    NumberedEntryIter::new(dir, base)
        .map(|entry| entry.number)
        .max()
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
        let builder = $crate::testdir_global_builder!();
        let module_path = ::std::module_path!();
        let test_name = $crate::extract_test_name(&module_path);
        let subdir_path = ::std::path::Path::new(&module_path.replace("::", "/")).join(&test_name);
        $crate::with_global_testdir(builder, move |tdir| {
            $crate::__private_create_cargo_pid_file(tdir.path());
            tdir.create_subdir(subdir_path)
        })
    }};
    ( ModuleScope ) => {{
        let builder = $crate::testdir_global_builder!();
        let module_path = ::std::module_path!();
        let subdir_path = ::std::path::Path::new(&module_path.replace("::", "/")).join("mod");
        $crate::with_global_testdir(builder, move |tdir| {
            $crate::__private_create_cargo_pid_file(tdir.path());
            tdir.create_subdir(subdir_path)
        })
    }};
    ( $e:expr ) => {{
        let builder = $crate::testdir_global_builder!();
        $crate::with_global_testdir(builder, move |tdir| {
            $crate::__private_create_cargo_pid_file(tdir.path());
            tdir.create_subdir($e)
        })
    }};
}

/// Returns the global [`TESTDIR_BUILDER`].
///
/// This has to be in macro code as it needs to extract `CARGO_PKG_NAME` of the package in
/// which this is called at build time.
#[macro_export]
macro_rules! testdir_global_builder {
    () => {{
        let pkg_name = String::from(::std::env!("CARGO_PKG_NAME"));
        $crate::TESTDIR_BUILDER.get_or_init(move || {
            let mut builder = $crate::NumberedDirBuilder::new(pkg_name);
            builder.reusefn($crate::__private_reuse_cargo);
            builder
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
    fn test_builder_create() {
        let parent = tempfile::tempdir().unwrap();
        let dir = NumberedDirBuilder::new(String::from("base"))
            .tmpdir_provider(|| parent.path().to_path_buf())
            .create();
        assert!(dir.path().is_dir());
        let root = dir
            .path()
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy();
        assert!(root.starts_with("testdir-of-"));
    }

    #[test]
    fn test_builder_root() {
        let parent = tempfile::tempdir().unwrap();
        let dir = NumberedDirBuilder::new(String::from("base"))
            .tmpdir_provider(|| parent.path().to_path_buf())
            .root("myroot")
            .create();
        assert!(dir.path().is_dir());
        let root = parent.path().join("myroot");
        assert_eq!(dir.path(), root.join("base-0"));
    }

    #[test]
    fn test_builder_user_root() {
        let parent = tempfile::tempdir().unwrap();
        let dir = NumberedDirBuilder::new(String::from("base"))
            .tmpdir_provider(|| parent.path().to_path_buf())
            .root("myroot-")
            .create();
        assert!(dir.path().is_dir());
        let root = dir
            .path()
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy();
        assert!(root.starts_with("myroot-"));
    }

    #[test]
    fn test_builder_set_parent() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("myparent");
        let dir = NumberedDirBuilder::new(String::from("base"))
            .set_parent(parent.clone())
            .create();
        assert!(dir.path().is_dir());
        assert_eq!(dir.path(), parent.join("base-0"));
    }

    #[test]
    fn test_builder_count() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path();
        let mut builder = NumberedDirBuilder::new(String::from("base"));
        builder.tmpdir_provider(|| parent.to_path_buf());
        builder.count(NonZeroU8::new(1).unwrap());

        let dir0 = builder.create();
        assert!(dir0.path().is_dir());

        let dir1 = builder.create();
        assert!(!dir0.path().is_dir());
        assert!(dir1.path().is_dir());
    }

    #[test]
    fn test_with_global_testdir() {
        let parent = tempfile::tempdir().unwrap();
        let mut builder = NumberedDirBuilder::new(String::from("crate"));
        builder.tmpdir_provider(|| parent.path().to_path_buf());
        builder.root("myroot");

        let dir: PathBuf = with_global_testdir(&builder, |tdir: &NumberedDir| {
            tdir.create_subdir("test_with_global_testdir")
        });
        let expected_path = parent
            .path()
            .join("myroot")
            .join("crate-0")
            .join("test_with_global_testdir");
        assert_eq!(dir, expected_path);
        assert!(dir.is_dir());

        builder.base(String::from("crate-name-already-initialised"));

        let dir: PathBuf = with_global_testdir(&builder, |tdir: &NumberedDir| {
            tdir.create_subdir("test_with_global_testdir")
        });
        let expected_path = parent
            .path()
            .join("myroot")
            .join("crate-0")
            .join("test_with_global_testdir-0");
        assert_eq!(dir, expected_path);
        assert!(dir.is_dir());
    }
}
