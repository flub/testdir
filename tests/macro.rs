use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use testdir::testdir;

static MOD_LEVEL: Lazy<PathBuf> = Lazy::new(|| testdir!(ModuleScope));

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

#[test]
fn test_string() {
    let val = testdir!("sub/dir");
    println!("{}", val.display());
    panic!("the end");
}

#[test]
fn test_path() {
    let val = testdir!(Path::new("sub/dir"));
    println!("{}", val.display());
    panic!("the end");
}

#[test]
fn test_pathbuf() {
    let val = testdir!(PathBuf::from("sub/dir"));
    println!("{}", val.display());
    panic!("the end");
}

#[test]
fn test_varname() {
    let path = Path::new("sub/dir");
    let val = testdir!(path);
    println!("{}", val.display());
    panic!("the end");
}

mod submodule {
    use super::*;

    static SUB_MOD: Lazy<PathBuf> = Lazy::new(|| testdir!(ModuleScope));

    #[test]
    fn test_test_scope() {
        let val: PathBuf = testdir!();
        println!("{}", val.display());
        panic!("the end");
    }

    #[test]
    fn test_module_scope() {
        println!("{}", SUB_MOD.display());
        panic!("the end");
    }
}
