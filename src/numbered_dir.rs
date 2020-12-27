//! The [`NumberedDir`] type and supporting code.

use std::fs;
use std::num::NonZeroU8;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::symlink as symlink_dir;
#[cfg(windows)]
use std::os::windows::fs::symlink_dir;

/// A sequentially numbered directory.
///
/// This struct represents a directory is a sequentially numbered list of directories.  It
/// allows creating the next sequential directory safely across processes or threads without
/// any coordination as well as cleanup of older directories.
///
/// The directory has a **parent** directory in which the numbered directory is created, as
/// well as a **base** which is used as the directory name to which to affix the number.
#[derive(Clone, Debug)]
pub struct NumberedDir {
    pub(crate) path: PathBuf,
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

    /// Returns the path of this numbered directory instance.
    pub fn path(&self) -> &Path {
        &self.path
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
    pub fn create_subdir(&self, rel_path: impl AsRef<Path>) -> PathBuf {
        let rel_path = rel_path.as_ref();
        assert!(rel_path.is_relative(), "not a relative path");
        let file_name = rel_path
            .file_name()
            .expect("subdir does not end in a file_name");

        if let Some(parent) = rel_path.parent() {
            let parent_path = self.path.join(parent);
            fs::create_dir_all(&parent_path).unwrap_or_else(|_| {
                panic!("Failed to create subdir parent: {}", parent_path.display())
            });
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
            fs::remove_dir_all(&path)
                .unwrap_or_else(|_| panic!("Failed to remove {}", path.display()));
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
pub(crate) struct NumberedEntry {
    pub(crate) number: u16,
    pub(crate) name: String,
}

#[derive(Debug)]
pub(crate) struct NumberedEntryIter {
    prefix: String,
    readdir: fs::ReadDir,
}

impl NumberedEntryIter {
    pub(crate) fn new(dir: impl AsRef<Path>, base: &str) -> Self {
        Self {
            prefix: format!("{}-", base),
            readdir: dir
                .as_ref()
                .read_dir()
                .unwrap_or_else(|_| panic!("Failed read_dir() on {}", dir.as_ref().display())),
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
}
