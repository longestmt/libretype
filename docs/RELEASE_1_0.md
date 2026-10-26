# LibreType 1.0 release checklist

Feature freeze accepted on 2026-10-26. Only release documentation, packaging and
fixes for defects in the frozen workflows belong in this milestone. A failing
check must identify a concrete defect; it does not reopen general feature work.

## Frozen scope

Ship the existing ghost-text display, word/full acceptance, mid-line preview,
dismissal, manual activation, instructions, exclusions, shortcuts, spelling,
emoji, alternatives, model selection, local context and statistics. Compatibility
claims remain limited to the tested workflows in [STATUS.md](STATUS.md).

Personalization remains optional, off by default and experimental. Its known
name extraction, spelling and sign-off inconsistencies are documented rather
than release blockers. Google Docs remains an optional experimental Chrome
companion. Unverified terminal/editor integrations carry no support guarantee.
New adapters, model/prompt experiments, wider layout coverage and additional
performance studies belong after 1.0. Cross-Mac synchronization is out of scope.

## Candidate acceptance

- [x] Freeze scope and label experimental capabilities in documentation/settings.
- [x] Package the final candidate and its matching complete-source archive;
  verify offline compilation/tests, notices, source manifest and signature.
- [x] Install build 66 at `~/Applications/LibreType.app`, retaining rollback;
  verify startup, model discovery and a local preview completion.
- [x] Renew macOS Accessibility approval and verify the keyboard listener.
- [ ] Run one short native acceptance pass: ghost text, Tab, Shift-Tab, Esc,
  normal Tab without a suggestion, and pause/resume in a disposable editor.
- [x] Inspect the changed settings labels for clipping.
- [ ] If the optional Docs companion is enabled, check its final packaged version in the disposable
  document; do not expand this into another compatibility survey.
- [x] Document first-run instructions and untested OS/hardware limits in
  [installation notes](INSTALLING.md).
- [ ] Verify launch-at-login behavior; do not claim a clean-machine test based
  on an existing development installation.
- [ ] Complete 1–2 days of ordinary-use feedback with no known unresolved
  crash, unintended insertion, privacy-boundary failure or core typing regression.

The candidate uses application version `1.0.0` with a distinct build number.
Until acceptance and distribution checks pass it is a **release candidate**,
not a stable release. The numeric bundle version alone is not release approval.

Candidate build 66 uses source revision
`0bcb1b1bd924ef24dfa70e9d7f508d96d20a6d22`. Its 12,849 archive hashes and 279
public-source files matched; empty-cache offline compilation, Rust tests (209
library / 250 application, with overlap), 18 packaging tests, 39 companion tests
and notice/signature checks passed. The exported app retained 145 notice files.
The local archive and zip checksums are:

```text
a8f1891cbcac3aa44c7c8834db6c726e707e139e4650cd3e5977d78497a92a2b  libretype-1.0.0-candidate-build66-source.tar.gz
964aa2376f0a0ff5fd40883c5c25078dbc3c4c1932655e934f64adbc02d4f088  LibreType-1.0.0-candidate-build66-macos-arm64.zip
```

The installation retained build 64 as rollback. App data files were byte-unchanged
after startup; history stayed off and locked. The preview produced a completion,
and both experimental labels fit their windows. Renewed Accessibility approval
and the keyboard listener are now active in the running build 66 process. Native
typing acceptance is pending. Offline rebuilding
does not establish bit-for-bit reproducibility. These results were recorded
after packaging, so the archive contains the checklist as it stood at its source
revision. Nothing has been published.

## Distribution

- [ ] Finish the contributor/provenance review recorded in
  [third-party review](../third_party/REVIEW.md).
- [ ] Choose and verify distribution signing. No valid certificate identity was
  available at the last check; local candidates are ad-hoc signed. Developer ID
  signing/notarization requires the corresponding credentials. If a deliberate
  source/developer release is chosen instead, document its installation limits.
- [ ] Publish the approved binary and matching source, checksums and concise
  installation/known-limit notes; verify the downloads. Follow
  [RELEASING.md](RELEASING.md). A private commit or local archive is not publication.

Final candidate acceptance and publication are the remaining milestone work.
Experimental feature improvements do not become new release gates unless they
cause a crash, unsafe edit or privacy failure in the shipping application.
