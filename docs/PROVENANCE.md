# Source provenance and release review

Reviewed for the 1.0 release on 2026-10-26. This records the development evidence
and licensed inputs; it does not claim a separated clean-room process or legal
clearance for every possible third-party right.

## Chronology and evidence

- On 2026-10-22, development included read-only inspection of a reference
  application package, package declarations, executable imports and native
  class/dispatch metadata. The private ledger and four retained evidence files
  record the inspected artifacts, hashes, tool outcomes and limits. Procedure
  access did not recover function pseudocode or a complete call graph. The
  ledger did not recover completion prompts or subscription-enforcement code.
- The original session log remains private. It records separate feature,
  Rust-core and native-integration task launches on 2026-10-22. Some stored
  message payloads are encrypted; the release review cannot reconstruct all
  per-agent inputs from those records. Shared development context precludes a
  claim of clean-room separation.
- An independent technical review of the 2026-10-23 snapshot identified missing
  notices, a transformed external test fixture, incomplete corresponding-source
  delivery and insufficient model-license disclosure. It found no recovered
  reference function implementation in the evidence it examined. That narrow
  finding is not a whole-program similarity analysis or proof of noninfringement.
- The application adopted GPL-3.0-only at the owner's direction. Existing MIT
  permissions on older copies were not withdrawn. The public Git history begins
  with a development snapshot on 2026-10-23; it is not the complete original
  development transcript.
- On 2026-10-24, the incomplete-notice buffer dependency was replaced by the
  documented licensed adapter. On 2026-10-25, the incomplete-notice block
  dependency and its Cocoa dependency chain were replaced by scoped system ABI
  definitions. See [the dependency review](../third_party/REVIEW.md) and
  [the binding patch record](../vendor/objc/SOURCE.md).
- The externally derived terminal fixture is excluded from publication. The
  current test constructs an in-project synthetic layout instead. Offline tests
  from the reviewed source archive verify that private research is not a build
  or test dependency.
- By 2026-10-26, the candidate's complete source had rebuilt offline with its
  locked dependencies and matching runtime source. Packaging checks verified
  application, dependency, data and runtime notices. Private build evidence and
  earlier artifacts remain excluded from the release manifest.

## Included third-party material

The exact Cargo resolution and retained notices are in
[`third_party/notices/inventory.json`](../third_party/notices/inventory.json).
It distinguishes runtime and build/procedural-macro reachability. The vendored
Objective-C binding patch, unchanged emoji data and curl license records have
their own source/notice files under `vendor/`. Required upstream attribution
remains intact. Apple system libraries are linked, not redistributed. No model
weights or Ollama binary are bundled; their separate terms are disclosed before
downloads.

## Authorship and scope

Development is AI-assisted and directed by the project owner. Current public Git
author records identify the owner; that metadata alone does not establish human
authorship of every line. The retained contributor attribution does not promise
copyright protection or enforceable copyleft over purely generated material.
GPL notices apply to copyrightable application-owned contributions to the extent
their rights holders can license them. Third-party materials retain their terms.

The owner has instructed GPLv3 licensing and release. Technical release review
covers the retained licenses, notices, build inputs and corresponding-source
delivery. It does not decide the enforceability of reference-software contract
terms, patent or trademark questions, or the copyright status of every generated
portion. Those questions are distinct from supplying the GPL release materials.
Original inspection/session evidence is retained privately rather than removed
or represented as proof of a stronger development separation.
