use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use testdir::testdir;

static MOD_LEVEL: Lazy<PathBuf> = Lazy::new(|| testdir!(ModuleScope));

#[test]
fn test_macro() {
    let val: PathBuf = testdir!();
    println!("{}", val.display());
    assert!(val.ends_with("r#macro/test_macro"));
}

#[test]
fn test_write() {
    let dir = testdir!();
    assert!(dir.ends_with("test_write"));
    let path = dir.join("hello.txt");
    std::fs::write(&path, "hi there").ok();
    assert!(path.exists());
}

#[test]
fn test_read() {
    let dir = testdir!();
    assert!(dir.ends_with("r#macro/test_read"));
    let path = dir.join("hello.txt");
    assert!(!path.exists());
}

#[test]
fn test_mod_level() {
    assert!(MOD_LEVEL.ends_with("r#macro/mod"));
}

#[test]
fn test_string() {
    let val = testdir!("sub/dir");
    println!("{}", val.display());
    assert!(val.ends_with("sub/dir"));
}

#[test]
fn test_path() {
    let val = testdir!(Path::new("sub/dir0"));
    println!("{}", val.display());
    assert!(val.ends_with("sub/dir0"));
}

#[test]
fn test_pathbuf() {
    let val = testdir!(PathBuf::from("sub/dir1"));
    println!("{}", val.display());
    assert!(val.ends_with("sub/dir1"));
}

#[test]
fn test_varname() {
    let path = Path::new("sub/dir2");
    let val = testdir!(path);
    println!("{}", val.display());
    assert!(val.ends_with("sub/dir2"));
}

#[test]
fn test_cargo_pid_created() {
    let root = testdir!("spam");
    println!("{}", root.display());
    let cargo_pid = root.join("../cargo-pid");
    assert!(cargo_pid.is_file());
}

mod submodule {
    use super::*;

    static SUB_MOD: Lazy<PathBuf> = Lazy::new(|| testdir!(ModuleScope));

    #[test]
    fn test_test_scope() {
        let val: PathBuf = testdir!();
        println!("{}", val.display());
        assert!(val.ends_with("r#macro/submodule/test_test_scope"));
    }

    #[test]
    fn test_module_scope() {
        println!("{}", SUB_MOD.display());
        assert!(SUB_MOD.ends_with("r#macro/submodule/mod"));
    }
}
