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

#![warn(missing_docs, missing_debug_implementations, clippy::all)]

use std::num::NonZeroU8;

use once_cell::sync::OnceCell;

mod builder;
mod numbered_dir;
mod testdir;

#[doc(hidden)]
pub mod private;

pub use builder::NumberedDirBuilder;
pub use numbered_dir::{NumberedDir, NumberedDirIter};

/// Default to build the `root` for [`NumberedDirBuilder`] and [`testdir!`] from: `testdir`.
pub const ROOT_DEFAULT: &str = "testdir";

/// The default number of test directories retained by [`NumberedDirBuilder`] and
/// [`testdir!`]: `8`.
pub const KEEP_DEFAULT: Option<NonZeroU8> = NonZeroU8::new(8);

/// **Private** The global [`NumberedDir`] instance used by [`with_testdir`].
///
/// Do not use this directly, use [`init_testdir!`] to initialise this.
#[doc(hidden)]
pub static TESTDIR: OnceCell<NumberedDir> = OnceCell::new();

/// Executes a function passing the global [`NumberedDir`] instance.
///
/// This is used by the [`testdir!`] macro to create subdirectories inside one global
/// [`NumberedDir`] instance for each test using [`NumberedDir::create_subdir`].  You may
/// use this for similar purposes.
///
/// Be aware that you should have called [`init_testdir!`] before calling this so that the
/// global testdir was initialised correctly.  Otherwise you will get a dummy testdir name.
///
/// # Panics
///
/// If there is not yet a global testdir initialised, see [`init_testdir!`], this could
/// panic while initialising it.
///
/// # Examples
///
/// ```
/// use testdir::{init_testdir, with_testdir};
///
/// init_testdir!();
/// let path = with_testdir(|dir| dir.create_subdir("some/path").unwrap());
/// assert!(path.is_dir());
/// assert!(path.ends_with("some/path"));
/// ```
pub fn with_testdir<F, R>(func: F) -> R
where
    F: FnOnce(&NumberedDir) -> R,
{
    let test_dir = TESTDIR.get_or_init(|| {
        let mut builder = NumberedDirBuilder::new(String::from("init_testdir-not-called"));
        builder.reusefn(private::reuse_cargo);
        let testdir = builder.create().expect("Failed to create testdir");
        private::create_cargo_pid_file(testdir.path());
        testdir
    });
    func(test_dir)
}
