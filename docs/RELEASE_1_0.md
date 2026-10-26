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
- [ ] Package the final candidate and its matching complete-source archive;
  verify offline compilation/tests, notices, source manifest and signature.
- [ ] Install that candidate at `~/Applications/LibreType.app`, retaining rollback;
  verify startup, model discovery and active Accessibility/keyboard listener.
- [ ] Run one short native acceptance pass: ghost text, Tab, Shift-Tab, Esc,
  normal Tab without a suggestion, and pause/resume in a disposable editor.
- [ ] Inspect the changed settings labels for clipping. If the optional Docs
  companion is enabled, check its final packaged version in the disposable
  document; do not expand this into another compatibility survey.
- [ ] Check first-run instructions and launch-at-login behavior. State any
  untested OS/hardware limits explicitly; do not claim a clean-machine test
  based on an existing development installation.
- [ ] Complete 1–2 days of ordinary-use feedback with no known unresolved
  crash, unintended insertion, privacy-boundary failure or core typing regression.

The candidate uses application version `1.0.0` with a distinct build number.
Until acceptance and distribution checks pass it is a **release candidate**,
not a stable release. The numeric bundle version alone is not release approval.

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
