//! The [`NumberedDirBuilder`].

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::num::NonZeroU8;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Error, Result};

use crate::{NumberedDir, KEEP_DEFAULT, ROOT_DEFAULT};

/// Builder to create a [`NumberedDir`].
///
/// While you can use [`NumberedDir::create`] directly this provides functionality to
/// specific ways of constructing and re-using the [`NumberedDir`].
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
/// can use [`NumberedDirBuilder::root`] or [`NumberedDirBuilder::user_root].  The temporary
/// directory provider can also be changed using [`NumberedDirBuilder::tmpdir_provider`].
///
/// If you simply want an absolute path as parent directory for the numbered directory use
/// the [`NumberedDirBuilder::set_parent`] function.
///
/// Sometimes you may have some external condition which signals that an existing numbered
/// directory should be re-used.  The [`NumberedDirBuilder::reusefn] can be used for this.
/// This is useful for example when running tests using `cargo test` and you want to use the
/// same numbered directory for the unit, integration and doc tests even though they all run
/// in different processes.  The [`testdir`] macro does this by storing the process ID of
/// the `cargo test` process in the numbered directory and comparing that to the parent
/// process ID of the current process.
///
/// # Creating the [`NumberedDir`]
///
/// The [`NumberedDirBuilder::create`] method will create a new [`NumberedDir`].
#[derive(Clone)]
pub struct NumberedDirBuilder {
    /// The current absolute path of the parent directory.  The last component is the
    /// current root.  This is the parent directory in which we should create the
    /// NumberedDir.
    parent: PathBuf,
    /// The base of the numbered dir, its name without the number suffix.
    base: String,
    /// The number of numbered dirs to keep around **after** the new directory is created.
    count: NonZeroU8,
    /// Function to determine whether to re-use a numbered dir.
    #[allow(clippy::type_complexity)]
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
    pub fn create(&self) -> Result<NumberedDir> {
        if !self.parent.exists() {
            fs::create_dir_all(&self.parent).context("Failed to create root directory")?;
        }
        if !self.parent.is_dir() {
            return Err(Error::msg("Path for root is not a directory"));
        }
        if let Some(ref reuse_fn) = self.reuse_fn {
            for numdir in NumberedDir::iterate(&self.parent, &self.base)? {
                if reuse_fn(numdir.path()) {
                    return Ok(numdir);
                }
            }
        }
        NumberedDir::create(&self.parent, &self.base, self.count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_create() {
        let parent = tempfile::tempdir().unwrap();
        let dir = NumberedDirBuilder::new(String::from("base"))
            .tmpdir_provider(|| parent.path().to_path_buf())
            .create()
            .unwrap();
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
            .create()
            .unwrap();
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
            .create()
            .unwrap();
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
            .create()
            .unwrap();
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

        let dir0 = builder.create().unwrap();
        assert!(dir0.path().is_dir());

        let dir1 = builder.create().unwrap();
        assert!(!dir0.path().is_dir());
        assert!(dir1.path().is_dir());
    }
}
