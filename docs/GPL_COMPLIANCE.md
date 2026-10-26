# GPLv3 release delivery review

Reviewed 2026-10-26 for LibreType 1.0.0, build 67. The owner chose **GPL-3.0-only**
and directed release. This is a technical licensing/source-delivery review, not
an opinion on all contractual, patent, trademark or authorship questions.

## Requirements and evidence

| Area | Release treatment |
| --- | --- |
| Application license | Complete GPLv3 text in `LICENSE`; GPL-3.0-only Cargo metadata, source headers, README and copyright notice. Earlier MIT permissions are not revoked. |
| Interactive notices | LT menu/settings license view states copyright, redistribution rights and no warranty, opens full licenses, and identifies the exact source release. The Docs popup has its own visible license/source disclosure. |
| Third-party notices | Exact lockfile and target-resolved inventory includes nested licenses, data notices and the matching Rust runtime notices. Runtime/build roles are distinguished. Required texts enter the app before signing. |
| Previously incomplete crate notices | The two affected dependencies were removed. The ownership adapter and scoped ABI replacements have retained source, modification records and tests. Upstream licenses on the remaining binding code are preserved. |
| Test fixture | The previously external snapshot is excluded. The distributed test constructs its synthetic layout locally and does not require private research files. |
| Corresponding source | The versioned release provides a complete-source archive beside the binary at no extra charge, with application source, Cargo.lock, locked dependency source, runtime source, build/install scripts and tests. |
| Modified builds | Build/install instructions and ad-hoc packaging require no project signing secret. No license/account server prevents use of a modified build. |
| Separate programs/models | Ollama and weights are not bundled. Model notices/terms are shown before explicit download; restricted model terms are not applied to the GPL application. |
| Publication hygiene | The allowlisted Git tree contains source, tests, tooling, documentation and notices. Private inspection/session data, app backups, writing captures and model weights are excluded. |
| Provenance/authorship | The factual chronology and limitations are recorded in `PROVENANCE.md`; retained attribution does not claim human authorship of every AI-generated line or a strict clean-room process. |

## Delivery mechanism

The selected delivery mechanism is GPLv3 section 6(d): equivalent access to the
binary and its corresponding source from the same versioned GitHub release.
The release notes link the complete-source asset directly beside the app; a
Cargo.lock-only GitHub snapshot is not substituted for that archive. Retain the
matching source while distributing the binary. There is no separate time-limited
written source offer, extra application EULA or subscription condition.

Build prerequisites and installation steps are supplied. No device hardware is
being transferred as part of this release, and no private application key is
needed to run modified source builds. If the distribution arrangement changes,
review any additional GPL installation-information obligations then.

## Review limits

The earlier independent review's technical notice, fixture, model-disclosure and
source-delivery findings are addressed by the evidence above. Its broader
provenance/ownership cautions are retained in the factual chronology and accurate
attribution; they are not represented as a guarantee of noninfringement or of
copyright protection over every generated portion. The available evidence does
not identify an unresolved exact-version license notice in the shipped graph.

Final archive build/tests, signature and asset-download results belong in the
[release record](RELEASE_1_0.md). Do not claim a check passed until its final
artifact was inspected.

The full governing terms are in [LICENSE](../LICENSE), also available through
[GNU](https://www.gnu.org/licenses/gpl-3.0.html) and
[SPDX](https://spdx.org/licenses/GPL-3.0-only.html). Relevant provisions include
sections 0–1, 4–6 and 10; this table does not replace those terms.
