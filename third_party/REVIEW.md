# Third-party notice review

Reviewed 2026-10-25 against the retained Cargo.lock and aarch64-apple-darwin
resolution. The 67-package inventory includes build tools and procedural macros;
see each package's `roles` in notices/inventory.json. No exact-version notice
files are missing from this inventory. All available nested legal files are
retained, including ring's BoringSSL, ISC, once_cell and fiat notices. Unicode
data notices and the exact installed Rust standard-library copyright report
are included. Apple system frameworks and dynamic system libcurl are not
redistributed by this source tree.

## Remaining release review

Confirm rights/contributor authority for application-owned contributions.
The retained copyright attribution does not assert exclusive human authorship
of every generated line. Preserve original development records privately.
A complete notice inventory is not a determination of rights to distribute;
public release still requires this review and the delivery checks in
[release instructions](../docs/RELEASING.md).

## Replaced dependencies

The `malloc_buf` 0.0.6 dependency has been removed from the lockfile and dependency
graph. Its later upstream license is not applied retroactively. The patched
`objc` source instead uses `malloced` 1.3.1 through a small adapter; the downloaded
crate includes its complete MIT and Apache-2.0 notices. The original `objc` MIT
notice and the modifications' GPL-3.0 text are retained. See
[source provenance](../vendor/objc/SOURCE.md) for the exact scope and validation.

The `block` 0.1.6 dependency and the `cocoa` dependency chain that retained it
have also been removed. The application uses its own small Rust definitions for
64-bit macOS geometry and the two-pointer callbacks needed by ScreenCaptureKit.
These definitions follow Apple's SDK geometry declarations and the documented
[Apple Blocks ABI](https://clang.llvm.org/docs/Block-ABI-Apple.html); allocation
and reference counting use the macOS runtime. Existing dependency interfaces
were inspected during the change; this is not a clean-room claim. No source
from the removed crates is included in these replacements.

An independent test-only Objective-C consumer checks both callback signatures,
copying, invocation on a worker thread after the Rust owner is dropped, and
final capture cleanup. Foundation checks geometry layout and message round
trips. These tests do not exercise live screen capture or establish support on
untested CPU architectures. Build 49 also passed a native Twinny ghost-display
and Tab-insertion check after Accessibility renewal. Live screen capture remains
unverified and disabled.

## Test provenance

The multiline terminal regression constructs its own synthetic layout with an
in-project test helper. It does not load an externally derived snapshot. The
earlier snapshot and development evidence are retained privately and excluded
from the public manifest. The gemoji data remains unchanged with its original
MIT notice and pinned source information.

## Reproduction

`python3 scripts/license_inventory.py --check` compares retained notice bytes,
package versions, lockfile digest and compiler identity to the local environment.
Unknown or missing notices require review; the script does not manufacture them.
