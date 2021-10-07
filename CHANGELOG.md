# Changelog

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
