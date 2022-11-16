//! The `testdir!()` macro and friends.
//!
//! Tests for this are almost exclusively in the integration tests at `tests/macro.rs`.

/// Creates a test directory at the requested scope.
///
/// This macro creates a new or re-uses an existing [`NumberedDir`] in the cargo target
/// directory.  It than creates the requested sub-directory within this [`NumberedDir`].
/// The path for this directory is returned as a [`PathBuf`].
///
/// For the typical `testdir!()` invocation in a test function this would result in
/// `target/testdir-$N/$CARGO_CRATE_NAME/module/path/to/test_function_name1.  A symbolic
/// link to the most recent [`NumberedDir`] is also created as `target/testdir-current ->
/// testdir-$N`.
///
/// **Reuse** of the [`NumberedDir`] is triggered when this process is being run as a
/// subprocess of Cargo, as is typical when running `cargo test`.  In this case the same
/// [`NumberedDir`] is re-used between all Cargo sub-processes, which means it is shared
/// between unittests, integration tests and doctests of the same test run.
///
/// The path within the numbered directory is created based on the context and how it is
/// invoked.  There are several ways to specify this:
///
/// * Use the scope of the current test function to create a unique and predictable
///   directory: `testdir!(TestScope)`.  This is the default when invoked as without any
///   arguments as well: `testdir!()`.  In this case the directory path will follow the crate
///   name and module path, ending with the test function name.  This also works in
///   integration and doctests.
///
/// * Use the scope of the current module: `testdir!(ModuleScope)`.  In this case the crate
///   name and module path is used, but with an additional final `mod` component.
///
/// * Directly provide the path using an expression, e.g. `testdir!("sub/dir").  This
///   expression will be passed to [`NumberedDir::create_subdir`] and thus must evaluate to
///   something which implements `AsRef<Path>`, e.g. a simple `"sub/dir"` can be used or
///   something more advanced evaluating to a path, usually [`Path`] or [`PathBuf`].
///
/// # Panics
///
/// If there is any problem with creating the directories or cleaning up old ones this will
/// panic.
///
/// # Examples
///
/// Inside a test function you can use the shorthand:
/// ```
/// use std::path::PathBuf;
/// use testdir::testdir;
///
/// let path0: PathBuf = testdir!();
/// ```
///
/// This is the same as invoking:
/// ```
/// # use testdir::testdir;
/// let path1 = testdir!(TestScope);
/// ```
/// These constructs can also be used in a doctest.
///
/// The module path is valid in any scope, so can be used together with [once_cell] (or
/// [lazy_static]) to share a common directory between different tests.
/// ```
/// use std::path::PathBuf;
/// use once_cell::sync::Lazy;
/// use testdir::testdir;
///
/// static TDIR: Lazy<PathBuf> = Lazy::new(|| testdir!(ModuleScope));
///
/// #[test]
/// fn test_module_scope() {
///     assert!(TDIR.ends_with("mod"));
/// }
/// ```
///
/// [lazy_static]: https://docs.rs/lazy_static
/// [`NumberedDir`]: crate::NumberedDir
/// [`PathBuf`]: std::path::PathBuf
#[macro_export]
macro_rules! testdir {
    () => {
        testdir!(TestScope)
    };
    ( TestScope ) => {{
        $crate::init_testdir!();
        let module_path = ::std::module_path!();
        let test_name = $crate::private::extract_test_name(&module_path);
        let subdir_path = ::std::path::Path::new(&module_path.replace("::", "/")).join(&test_name);
        $crate::with_testdir(move |tdir| {
            tdir.create_subdir(subdir_path)
                .expect("Failed to create test-scoped sub-directory")
        })
    }};
    ( ModuleScope ) => {{
        $crate::init_testdir!();
        let module_path = ::std::module_path!();
        let subdir_path = ::std::path::Path::new(&module_path.replace("::", "/")).join("mod");
        $crate::with_testdir(move |tdir| {
            tdir.create_subdir(subdir_path)
                .expect("Failed to create module-scoped sub-directory")
        })
    }};
    ( $e:expr ) => {{
        $crate::init_testdir!();
        $crate::with_testdir(move |tdir| {
            tdir.create_subdir($e)
                .expect("Failed to create sub-directory")
        })
    }};
}

/// Initialises the global [`NumberedDir`] used by the [`testdir`] macro.
///
/// This macro is implicitly called by the [`testdir`] macro to initialise the global
/// [`NumberedDir`] instance inside the cargo target directory.  It must be called before
/// any call to [`with_testdir`](crate::with_testdir) to ensure this is initialised.
///
/// # Examples
///
/// ```
/// use testdir::{init_testdir, with_testdir};
///
/// init_testdir!();
/// let path = with_testdir(|dir| dir.create_subdir("some/subdir").unwrap());
/// assert!(path.is_dir());
/// assert!(path.ends_with("some/subdir"));
/// ```
///
/// [`NumberedDir`]: crate::NumberedDir
#[macro_export]
macro_rules! init_testdir {
    () => {{
        $crate::TESTDIR.get_or_init(move || {
            let metadata = $crate::private::cargo_metadata::MetadataCommand::new()
                .exec()
                .expect("cargo metadata failed");
            // let pkg_name = String::from(::std::env!("CARGO_PKG_NAME"));
            let pkg_name = "testdir";
            let mut builder = $crate::NumberedDirBuilder::new(pkg_name.to_string());
            builder.set_parent(metadata.target_directory.into());
            builder.reusefn($crate::private::reuse_cargo);
            let testdir = builder.create().expect("Failed to create testdir");
            $crate::private::create_cargo_pid_file(testdir.path());
            testdir
        })
    }};
}
