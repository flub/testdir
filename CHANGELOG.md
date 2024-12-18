# Changelog

## v0.9.3

- Specify an MSRV in Cargo.toml, checked in CI.
- The minimal versions for dependencies are tweaked and now checked in CI.

## v0.9.2

- Pin cargo-platform dependency so we do not exceed MSRV of 0.76.

## v0.9.1

- Fix windows support up a bit.  Previously it would not detect the
  correct cargo parent process and thus cycle through numbered
  directories too fast.  Also allow not finding the test name since
  doc tests on windows do not work.

## v0.9.0

- Support for cargo-nextest.

## v0.8.1

- When cleaning up old numbered directories if the entry is not found
  this error is ignored.  This is possible if multiple cleanups are
  racing each other.

## v0.8.0

- NumberedDir::create_subdir will no longer ensure to always create a
  new subdirectory.  Now if it is called with the same argument again
  the previously existing directory is reused.  This in particular
  means that calling the testdir!() macro multiple times inside a test
  will always give you back the same identical directory instead of a
  different one each time.

## v0.7.3

- Fallback to directory of test binary if cargo-metadata is not
  available.  This can be the case in some restricted environments,
  like phones when using cargo-dinghy.

## v0.7.2

- Made removal of outdated -current symlink optional: this can often
  fail on windows.  The symlink is now best effort.

## v0.7.1

- Fix testdir!() macro to call itself using $crate:: prefix so it does
  not rely on the testdir module being imported.

## v0.7.0

- Migrated dependency to get ppid and executable names from psutil to
  sysinfo, psutil has insufficient mac support.

## v0.6.0

- Migrated dependecy to get ppid and executable names from heim to
  psutil.
- Updated to edition 2021.
- Migrate to github.
- Fix bug for cargo_metadata dependency in macro.
- Always use "testdir" as basename instead of the package name since
  the tempdir is now created in the package's target directory.

## v0.5.1

### Changes

- Updated the README.

## v0.5.0

### Changes

- The default location for `testdir!()` is now in the cargo target
  directory instead of the system-provided temporary directory.

## v0.4.0

### Changes

- Most of the public API now returns Results where appropriate instead
  of panicking.  The macros keep panicing for convenience.

## v0.3.1

### Changes

- Cargo.toml now links to the docs on docs.rs.

## v0.3.0

### Changes

- Re-uses the NumberedDir instance across different Cargo
  subprocesses, this means only one instance is used for all the
  unittests, integration tests and doc tests of a single `cargo test`
  invocation.
