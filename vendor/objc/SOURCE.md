# Objective-C compatibility source

Base: `objc` 0.2.7 from its crates.io archive. The original MIT license is retained
in `LICENSE.txt`. Original Rust source files are retained except for the module
declaration in `src/lib.rs` and four raw-pointer borrows in `src/macros.rs`; no
generated binaries or registry cache files are included.

LibreType replaces the `malloc_buf` 0.0 dependency with a small GPL-3.0-only adapter
over `malloced` 1.3.1. The adapter is in `src/malloc_buf.rs`; its license text is
in `COPYING-GPL-3.0`. The package declares both applicable licenses. All original
Objective-C runtime declarations remain unchanged. Message macros use a raw
borrow instead of creating a reference to the receiver: Objective-C permits nil
receivers, while Rust references must be non-null. The original nil-message unit
test aborted on the current Rust toolchain before this change. These four edits
are GPL-3.0-only modifications, with the original MIT attribution retained.

The adapter preserves the copy-array ownership API used by this library. Null
empty arrays have no owner; non-null pointers transfer ownership to `Malloced`.
Non-null zero-length allocations are still owned and released. Only Copy element
types can be adopted, matching the existing call sites' references and bytes.

The original dependency's source/API and the successor implementation were read
during investigation. This is a documented compatibility change, not a claim of
strict clean-room development. No source from the replaced `malloc_buf` crate is
included in this adapter.

This patched copy is used only by LibreType through Cargo's local patch facility;
it is not an upstream release of `objc`.

Validation on the current Apple Silicon toolchain: 25 patched-library unit tests
and seven documentation tests pass, including the original nil-message test and
new null/empty/allocation ownership cases. LibreType's 229 deterministic tests,
strict application-level Clippy check and 11 packaging/publication tests pass.
Application and dependency tests also pass from the reviewed public-source
export. Build 49 passed a native Twinny ghost-display and Tab-insertion check
after Accessibility renewal; this is not a claim of universal native coverage.

Base crate archive SHA-256: `915b1b472bc21c53464d6c8461c9d3af805ba1ef837e1cac254428f4a77177b1`.
Recorded upstream VCS commit: `858f92f7c11deb4cd95fe8c8b58ff8e1a6758f5e`.
