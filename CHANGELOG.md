# Changelog

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
