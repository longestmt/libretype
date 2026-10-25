# Third-party notice review

Reviewed 2026-10-24 against the retained Cargo.lock and aarch64-apple-darwin
resolution. The 76-package inventory includes build tools and procedural macros;
see each package's `roles` in notices/inventory.json. All available nested legal
files are retained, including ring's BoringSSL, ISC, once_cell and fiat notices.
Unicode data notices and the exact installed Rust standard-library copyright
report are included. Apple system frameworks and dynamic system libcurl are not
redistributed by this source tree.

## Unresolved release gates

- **block 0.1.6:** the exact crate manifest declares MIT and Steven Sheldon as
  author. The upstream `SSheldon/rust-block` repository, including tag `0.1.6`,
  has no standalone copyright/license file. Obtain an authoritative notice or
  replace this dependency with an appropriately licensed implementation.
- Confirm rights/contributor authority for application-owned contributions.
  The retained copyright attribution does not assert exclusive human authorship
  of every generated line. Preserve original development records privately.

These are unresolved exact-notice/provenance questions, not findings that the
packages are necessarily incompatible. Public binary release stays gated.

## Replaced buffer dependency

The `malloc_buf` 0.0.6 dependency has been removed from this candidate's lockfile
and dependency graph. Its later upstream license is not applied retroactively.
The patched `objc` source instead uses `malloced` 1.3.1 through a small adapter;
the downloaded crate includes its complete MIT and Apache-2.0 notices. The original
`objc` MIT notice and the new modifications' GPL-3.0 text are retained. See
[source provenance](../vendor/objc/SOURCE.md) for the exact scope and validation.
This closes that dependency's missing-notice issue in the candidate, not the
separate block notice or contributor-authority gates. Installed-app validation
is still required before adopting the candidate.

## Test provenance

The multiline terminal regression now constructs its own synthetic layout with
an in-project test helper. It no longer loads an externally derived snapshot.
The earlier snapshot and development evidence are retained privately and excluded
from the public manifest. The gemoji data remains unchanged with its original
MIT notice and pinned source information.

## Reproduction

`python3 scripts/license_inventory.py --check` compares retained notice bytes,
package versions, lockfile digest and compiler identity to the local environment.
Unknown or missing notices require review; the script does not manufacture them.
Upstream repository references:

- https://github.com/SSheldon/rust-block/tree/0.1.6
- https://github.com/SSheldon/malloc_buf/tree/0.0.6
- https://github.com/SSheldon/malloc_buf/commit/d9a3e53
