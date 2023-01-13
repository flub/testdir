//! Tests which do not import the module.

#[test]
fn test_simple() {
    let path = testdir::testdir!();
    println!("{}", path.display());
    assert!(path.ends_with("no_import/test_simple"));
}
