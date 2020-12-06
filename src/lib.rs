// /tmp/rs-test-of-flub/byte_pool-N/module/path/test_name/
// /tmp/rs-test-of-flub/
// /tmp/rs-test-of-$user/
// /tmp/rs-test-of-$user/$crate_name-$N/
// /tmp/rs-test-of-$user/$crate_name-current -> $crate_name-$N
// /tmp/$root_dir/$base_name-$N/
// $max_N

use std::fs;
use std::num::NonZeroU8;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::symlink as symlink_dir;
#[cfg(windows)]
use std::os::windows::fs::symlink_dir;

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
}

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
    // Safety: user can freely escape
    // Performance: dozens of conflicts is fine, but don't do hundreds
    pub fn subdir(&self, rel_path: &Path) -> PathBuf {
        assert!(rel_path.is_relative(), "not a relative path");
        let file_name = rel_path
            .file_name()
            .expect("subdir does not end in a file_name");
        let mut full_path = self.path.join(&rel_path);

        for i in 0..u16::MAX {
            if full_path.exists() {
                let mut new_file_name = file_name.to_os_string();
                new_file_name.push(format!("-{}", i));
                full_path.set_file_name(new_file_name);
            }
        }
        if full_path.exists() {
            panic!("subdir conflict: all filename alternatives exhausted");
        }

        fs::create_dir_all(&full_path)
            .expect(&format!("Failed to create subdir {}", rel_path.display()));
        full_path
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

        let sub = dir.subdir(Path::new("sub"));
        assert_eq!(sub, dir.path().join("sub"));
        assert!(sub.is_dir());

        let sub_0 = dir.subdir(Path::new("sub"));
        assert_eq!(sub_0, dir.path().join("sub-0"));
        assert!(sub_0.is_dir());
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
}
