use std::path::PathBuf;

use once_cell::sync::Lazy;
use testdir::testdir;

static MOD_LEVEL: Lazy<PathBuf> = Lazy::new(|| testdir!());

#[test]
fn test_write() {
    let dir = testdir!();
    let path = dir.join("hello.txt");
    std::fs::write(&path, "hi there").ok();
    assert!(path.exists());
}

#[test]
fn test_read() {
    let dir = testdir!();
    let path = dir.join("hello.txt");
    assert!(!path.exists());
}

#[test]
fn test_mod_level() {
    println!("{}", MOD_LEVEL.display());
    panic!("the end");
}

#[test]
fn test_macro() {
    let val: PathBuf = testdir!();
    println!("{}", val.display());
    panic!("the end");
}

mod submodule {
    use super::*;

    #[test]
    fn test_name() {
        let val: PathBuf = testdir!();
        println!("{}", val.display());
        panic!("the end");
    }
}
