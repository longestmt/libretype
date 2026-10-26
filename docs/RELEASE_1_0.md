# LibreType 1.0 release record

The owner froze features on 2026-10-26 and then set the release objective:
release version 1.0 on GitHub with GPLv3 requirements met. The publication target
is `longestmt/libretype`, tag `v1.0.0`, version 1.0.0, build 67. No further feature
work belongs in this release.

## Release checks

- [x] Verified typing candidate: build 66 passed the user-run Chrome checks for
  display, word/full acceptance, Esc dismissal, Tab navigation and pause/resume.
- [x] Docs candidate: build 66 / companion 0.1.9 passed display and Shift-Tab
  insertion after the document was reloaded following the extension update.
  Native diagnostics matched text/caret geometry and verified insertion.
- [x] Experimental features and unverified integration/platform limits are
  disclosed. No cross-Mac synchronization is included.
- [x] Exact-version dependency notices, GPL identifiers, source provenance,
  contributor attribution and separate model terms reviewed. See
  [GPL compliance](GPL_COMPLIANCE.md) and [provenance](PROVENANCE.md).
- [x] App source location and companion license/warranty notice prepared.
- [x] Build and sign build 67, export its exact complete source, and verify the
  final archive offline with empty Cargo cache, required tests and notice checks.
- [x] Stage the exact final app, complete source and checksums at `v1.0.0`.
- [x] Publish the reviewed source/release on GitHub and verify equivalent public
  access and checksums for the binary and corresponding source.

## Publication result

Published on 2026-10-26 at 21:11:29 UTC with the owner's explicit approval to
make the repository public. [Version 1.0.0](https://github.com/longestmt/libretype/releases/tag/v1.0.0)
is a published stable release, targeting source commit
`0df7831069960074d6cceb3e71986925b9e3d872` and application build 67.

The final archive's 12,853 manifest hashes and 283 application/source files
matched. Empty-cache offline compilation, 209 library / 250 application tests
(with overlap), 18 packaging tests, 39 companion tests, notice inventory and
strict ad-hoc signature checks passed; 145 bundled notice files were verified.
The final popup license notice was also rendered and inspected. The installed
development app was not replaced during release packaging.

Anonymous GitHub API requests confirmed public repository access, a non-draft,
non-prerelease release, and the matching tag commit. Anonymous downloads of the
app, complete source, checksum file and verification report all matched local
hashes and GitHub asset digests. License bytes in the downloaded app/source and
the archive's dependency-source manifest were checked. Temporary downloads were
removed after verification. The exact verification report is a release asset.

| Asset | SHA-256 |
| --- | --- |
| `LibreType-1.0.0-macos-arm64.zip` | `89e35889688e78b21786572f69118b40a45decbbe8af2b6937c828f6d4a95cb2` |
| `libretype-1.0.0-source.tar.gz` | `72e66ab0108a2608232399e93468c143caa18341433b3a0a6b75b8f857860102` |

This result was recorded after publication; the immutable tagged source retains
the preparation checklist from its build commit. The release verification asset
and this record supply the completed check results without changing that tag or
the published artifacts.

## Distribution choice

The owner has no Apple Developer account. This is a source-first release with an
optional ad-hoc signed binary; Developer ID signing/notarization is not a release
requirement. Its downloaded-installation limits are disclosed. The license and
source build/install scripts permit modified builds without a private signing
key, license server or application usage restriction.

No clean-machine, actual login-launch or broad ordinary-use verification is
claimed. These known coverage limits remain visible in the release notes; they
are not new feature or release gates for the owner's requested 1.0 publication.
No known failure of the verified core typing workflows is being waived.

## Candidate evidence retained

Build 66 was compiled from `0bcb1b1bd924ef24dfa70e9d7f508d96d20a6d22`.
Its complete-source archive passed 12,849 manifest hash checks, matching all 279
public files at that revision, and rebuilt offline. Rust suites passed 209 library
and 250 application tests (with overlap); 18 packaging and 39 companion tests
passed. The rebuilt bundle verified its ad-hoc signature and 145 notice files.
This established buildability, not bit-for-bit reproducibility.

The private candidate draft includes its app, source and checksum assets. All
three were downloaded back and matched local hashes and GitHub asset digests.
The current source/release branch carries subsequent documentation and legal UI
changes; those changes require a new final archive, rather than relabeling the
candidate archive as matching build 67. Candidate evidence remains private/local
or in the unpublished candidate draft; it is not a substitute for final assets.
