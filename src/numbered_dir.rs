//! The [`NumberedDir`] type and supporting code.

use std::fs;
use std::num::NonZeroU8;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::symlink as symlink_dir;
#[cfg(windows)]
use std::os::windows::fs::symlink_dir;

use anyhow::{Context, Error, Result};

/// A sequentially numbered directory.
///
/// This struct represents a directory is a sequentially numbered list of directories.  It
/// allows creating the next sequential directory safely across processes or threads without
/// any coordination as well as cleanup of older directories.
///
/// The directory has a **parent** directory in which the numbered directory is created, as
/// well as a **base** which is used as the directory name to which to affix the number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumberedDir {
    path: PathBuf,
    /// The **base**, could also be extracted from `path`, needs to remain consistent.
    base: String,
    /// The number, could also be extracted from `path`, needs to remain consistent.
    number: u16,
}

impl NumberedDir {
    /// Creates the next sequential numbered directory.
    ///
    /// The directory will be created inside `parent` and will start with the name given in
    /// `base` to which the next available number is suffixed.
    ///
    /// If there are concurrent directories being created this will retry incrementing the
    /// number up to 16 times before giving up.
    ///
    /// The `count` specifies the total number of directories to leave in place, including
    /// the newly created directory.  Other previous directories with all their files and
    /// subdirectories are recursively removed.  Care is taken to avoid removing new
    /// directories concurrently created by parallel invocations in other threads or
    /// processes..
    pub fn create(parent: impl AsRef<Path>, base: &str, count: NonZeroU8) -> Result<Self> {
        if base.contains('/') || base.contains('\\') {
            return Err(Error::msg("base must not contain path separators"));
        }
        fs::create_dir_all(&parent).context("Could not create parent")?;
        let next_count = match current_entry_count(&parent, base) {
            Some(current_count) => {
                remove_obsolete_dirs(&parent, base, current_count, u8::from(count) - 1)?;
                current_count.wrapping_add(1)
            }
            None => 0,
        };
        create_next_dir(&parent, base, next_count)
    }

    /// Returns an iterator over all [`NumberedDir`] entries in a parent directory.
    ///
    /// This iterator can be used to get access to existing [`NumberedDir`] directories
    /// without creating a new one.
    pub fn iterate(parent: impl AsRef<Path>, base: &str) -> Result<NumberedDirIter> {
        NumberedDirIter::try_new(parent, base)
    }

    /// Returns the path of this numbered directory instance.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the **base** of this [`NumberedDir`] instance.
    ///
    /// The **base** is the name of the final [`NumberedDir::path`] component without the
    /// numbered suffix.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Returns the number suffix of this [`NumberedDir`] instance.
    ///
    /// The number is the suffix of the final component of [`NumberedDir::path`].
    pub fn number(&self) -> u16 {
        self.number
    }

    /// Creates a new subdirecotry within this numbered directory.
    ///
    /// This function will always create a new subdirecotry, if such a directory already
    /// exists the last component will get an incremental number suffix.
    ///
    /// # Limitations
    ///
    /// Only up to [`u16::MAX`] numbered suffixes are created so this is the maximum number
    /// of "identically" named directories that can be created.  Creating so many
    /// directories will become expensive however as the suffixes are linearly searched for
    /// the next available suffix.  This is not meant for a high number of conflicting
    /// subdirectories, if this is required ensure the `rel_path` passed in already avoids
    /// conflicts.
    ///
    /// There is no particular safety from malicious input, the numbered directory can be
    /// trivially escaped using the parent directory location: `../somewhere/else`.
    pub fn create_subdir(&self, rel_path: impl AsRef<Path>) -> Result<PathBuf> {
        let rel_path = rel_path.as_ref();
        if !rel_path.is_relative() {
            return Err(Error::msg(format!(
                "Not a relative path: {}",
                rel_path.display()
            )));
        }
        let file_name = rel_path.file_name().ok_or_else(|| {
            Error::msg(format!(
                "Subdir does not end in a filename: {}",
                rel_path.display()
            ))
        })?;

        if let Some(parent) = rel_path.parent() {
            let parent_path = self.path.join(parent);
            fs::create_dir_all(&parent_path).with_context(|| {
                format!("Failed to create subdir parent: {}", parent_path.display())
            })?;
        }

        let mut full_path = self.path.join(rel_path);
        for i in 0..u16::MAX {
            match fs::create_dir(&full_path) {
                Ok(_) => {
                    return Ok(full_path);
                }
                Err(_) => {
                    let mut new_file_name = file_name.to_os_string();
                    new_file_name.push(format!("-{}", i));
                    full_path.set_file_name(new_file_name);
                }
            }
        }
        Err(Error::msg(
            "subdir conflict: all filename alternatives exhausted",
        ))
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
fn remove_obsolete_dirs(dir: impl AsRef<Path>, base: &str, current: u16, keep: u8) -> Result<()> {
    let oldest_to_keep = current.wrapping_sub(keep as u16).wrapping_add(1);
    let oldest_to_delete = current.wrapping_add(u16::MAX / 2);
    assert!(oldest_to_keep != oldest_to_delete);

    for numdir in NumberedDir::iterate(&dir, base)? {
        if (oldest_to_keep > oldest_to_delete
            && (numdir.number < oldest_to_keep && numdir.number >= oldest_to_delete))
            || (oldest_to_keep < oldest_to_delete
                && (numdir.number < oldest_to_keep || numdir.number >= oldest_to_delete))
        {
            fs::remove_dir_all(numdir.path())
                .with_context(|| format!("Failed to remove {}", numdir.path().display()))?;
        }
    }

    Ok(())
}

/// Attempt to create the next numbered directory.
///
/// The directory will be placed in `dir` and its name composed of the `base` and
/// `next_count`.  If this directory can not be created it is assumed another process
/// created it already and the count is increased and tried again.  This is repeated maximum
/// 16 times after which this gives up.
///
/// Once the directory is created the `-current` symlink is also created.
fn create_next_dir(dir: impl AsRef<Path>, base: &str, mut next_count: u16) -> Result<NumberedDir> {
    let mut last_err = None;
    for _i in 0..16 {
        let name = format!("{}-{}", base, next_count);
        let path = dir.as_ref().join(name);
        match fs::create_dir(&path) {
            Ok(_) => {
                let current = dir.as_ref().join(format!("{}-current", base));
                if current.exists() {
                    fs::remove_file(&current).with_context(|| {
                        format!("Failed to remove obsolete {}-current symlink", base)
                    })?;
                }
                // Could be racing other processes, should not fail
                symlink_dir(&path, &current).ok();
                return Ok(NumberedDir {
                    path,
                    base: base.to_string(),
                    number: next_count,
                });
            }
            Err(err) => {
                next_count = next_count.wrapping_add(1);
                last_err = Some(err);
            }
        }
    }
    Err(Error::new(last_err.expect("no last error")).context("Failed to create numbered dir"))
}

fn current_entry_count(dir: impl AsRef<Path>, base: &str) -> Option<u16> {
    NumberedDirIter::try_new(dir, base)
        .ok()?
        .map(|entry| entry.number)
        .max()
}

/// Iterator of [`NumberedDir`] entries.
///
/// This will iterate over all [`NumberedDir`] entries in a parent directory with a given
/// base name.  It can be created using [`NumberedDir::iterate`].
#[derive(Debug)]
pub struct NumberedDirIter {
    /// The **base** plus the hyphen of the [`NumberedDir`] we are iterating over.
    prefix: String,
    /// Iterator of directory entries in which to look for our [`NumberedDir`] instances.
    readdir: fs::ReadDir,
}

impl NumberedDirIter {
    fn try_new(dir: impl AsRef<Path>, base: &str) -> Result<Self> {
        Ok(Self {
            prefix: format!("{}-", base),
            readdir: dir
                .as_ref()
                .read_dir()
                .with_context(|| format!("Failed read_dir() on {}", dir.as_ref().display()))?,
        })
    }
}

impl Iterator for NumberedDirIter {
    type Item = NumberedDir;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let mut dirent = self.readdir.next()?;
            while dirent.is_err() {
                dirent = self.readdir.next()?;
            }
            let dirent = dirent.ok()?;
            let os_name = dirent.file_name();

            // We only work with valid UTF-8 entry names, so skip any names which are not.
            let count = os_name
                .to_str()
                .and_then(|name| name.strip_prefix(&self.prefix))
                .and_then(|suffix| suffix.parse::<u16>().ok());
            if let Some(count) = count {
                return Some(NumberedDir {
                    path: dirent.path(),
                    base: self
                        .prefix
                        .strip_suffix('-')
                        .unwrap_or(&self.prefix)
                        .to_string(),
                    number: count,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_numbered_creation() {
        let parent = tempfile::tempdir().unwrap();
        let dir_0 = NumberedDir::create(parent.path(), "base", NonZeroU8::new(3).unwrap()).unwrap();
        assert_eq!(dir_0.path(), parent.path().join("base-0"));
        assert!(dir_0.path().is_dir());
    }

    #[test]
    fn test_numberd_creation_multiple() {
        let parent = tempfile::tempdir().unwrap();

        let dir_0 = NumberedDir::create(parent.path(), "base", NonZeroU8::new(3).unwrap()).unwrap();
        assert_eq!(dir_0.path(), parent.path().join("base-0"));
        assert!(dir_0.path().is_dir());

        let dir_1 = NumberedDir::create(parent.path(), "base", NonZeroU8::new(3).unwrap()).unwrap();
        assert_eq!(dir_1.path(), parent.path().join("base-1"));
        assert!(dir_0.path().is_dir());
        assert!(dir_1.path().is_dir());

        let dir_2 = NumberedDir::create(parent.path(), "base", NonZeroU8::new(3).unwrap()).unwrap();
        assert_eq!(dir_2.path(), parent.path().join("base-2"));
        assert!(dir_0.path().is_dir());
        assert!(dir_1.path().is_dir());
        assert!(dir_2.path().is_dir());

        let dir_3 = NumberedDir::create(parent.path(), "base", NonZeroU8::new(3).unwrap()).unwrap();
        assert_eq!(dir_3.path(), parent.path().join("base-3"));
        assert!(!dir_0.path().exists());
        assert!(dir_1.path().is_dir());
        assert!(dir_2.path().is_dir());
        assert!(dir_3.path().is_dir());

        let dir_4 = NumberedDir::create(parent.path(), "base", NonZeroU8::new(3).unwrap()).unwrap();
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
        let dir_0 = NumberedDir::create(parent.path(), "base", NonZeroU8::new(3).unwrap()).unwrap();
        assert_eq!(dir_0.path(), parent.path().join("base-0"));
        assert!(dir_0.path().is_dir());

        let current = fs::read_link(parent.path().join("base-current")).unwrap();
        assert_eq!(dir_0.path(), current);

        let dir_1 = NumberedDir::create(parent.path(), "base", NonZeroU8::new(3).unwrap()).unwrap();
        assert_eq!(dir_1.path(), parent.path().join("base-1"));
        assert!(dir_0.path().is_dir());
        assert!(dir_1.path().is_dir());

        let current = fs::read_link(parent.path().join("base-current")).unwrap();
        assert_eq!(dir_1.path(), current);
    }

    #[test]
    fn test_numbered_subdir() {
        let parent = tempfile::tempdir().unwrap();
        let dir = NumberedDir::create(parent.path(), "base", NonZeroU8::new(3).unwrap()).unwrap();

        let sub = dir.create_subdir(Path::new("sub")).unwrap();
        assert_eq!(sub, dir.path().join("sub"));
        assert!(sub.is_dir());

        let sub_0 = dir.create_subdir(Path::new("sub")).unwrap();
        assert_eq!(sub_0, dir.path().join("sub-0"));
        assert!(sub_0.is_dir());
    }

    #[test]
    fn test_numbered_subdir_nested() {
        let parent = tempfile::tempdir().unwrap();
        let dir = NumberedDir::create(parent.path(), "base", NonZeroU8::new(3).unwrap()).unwrap();

        let sub = dir.create_subdir(Path::new("one/two")).unwrap();
        assert_eq!(sub, dir.path().join("one/two"));
        assert!(dir.path().join("one").is_dir());
        assert!(dir.path().join("one").join("two").is_dir());
    }

    #[test]
    fn test_iter() {
        let parent = tempfile::tempdir().unwrap();
        fs::write(parent.path().join("oops"), "not a numbered dir").unwrap();

        let dir0 = NumberedDir::create(parent.path(), "base", NonZeroU8::new(3).unwrap()).unwrap();
        let dir1 = NumberedDir::create(parent.path(), "base", NonZeroU8::new(3).unwrap()).unwrap();
        let dirs = vec![dir0, dir1];

        for numdir in NumberedDir::iterate(parent.path(), "base").unwrap() {
            assert!(dirs.contains(&numdir));
        }
    }
}
