# LibreType 1.0.0

A source-first macOS typing assistant with local suggestions through a separately
installed Ollama service. No subscription, account or usage quota.

Application version **1.0.0**, build **67**. This release keeps build 66's verified
typing engine and adds the final source location, visible companion license
notice, and release/provenance documentation. Companion 0.1.10 changes the popup
notice only; its completion/setup scripts are unchanged from tested 0.1.9.

## Downloads and corresponding source

- **[Complete corresponding source](https://github.com/longestmt/libretype/releases/download/v1.0.0/libretype-1.0.0-source.tar.gz)** — application, locked dependencies, matching Rust runtime source, build/install tooling, tests and notices.
- **[Apple-silicon app](https://github.com/longestmt/libretype/releases/download/v1.0.0/LibreType-1.0.0-macos-arm64.zip)** — ad-hoc signed; **not Developer ID signed or notarized**.
- **[SHA-256 checksums](https://github.com/longestmt/libretype/releases/download/v1.0.0/SHA256SUMS.txt)** — verify downloads from this trusted release.

The complete-source archive is supplied at no charge alongside the binary.
GitHub's automatically generated source snapshots do not include the vendored
build inputs; use the complete-source asset above to reproduce this release.
The compiler version is recorded in `RELEASE.json`. Ollama and model weights are
installed separately and retain their own terms.

Application-owned source and documentation are **GPL-3.0-only**, without warranty.
Third-party materials retain their licenses and attribution. Full license and
notices are included in the app and source archive. See
[GPL delivery review](https://github.com/longestmt/libretype/blob/v1.0.0/docs/GPL_COMPLIANCE.md)
and [installation instructions](https://github.com/longestmt/libretype/blob/v1.0.0/docs/INSTALLING.md).

## Features

Inline ghost text; Tab word acceptance; Shift-Tab full acceptance; mid-line
previews; dismissal and manual activation; pause/resume; app/site instructions
and exclusions; spelling, emoji, alternatives and synonyms; local model selection;
optional clipboard/screen context; accepted-word counts. Google Docs support and
encrypted writing-history personalization are optional and experimental.

## Validation and known limits

Build 66 passed the user-run native Chrome checks for display, word/full
acceptance, dismissal, normal Tab navigation and pause/resume. The optional
Docs companion also passed display/insertion after a document reload following
its extension update. The final release's changes concern legal notices and
source delivery, not typing behavior. Offline source validation and packaged
notice checks are recorded with the release verification results.

Basic workflows have been exercised in TextEdit, Signal, Gmail, Discord, Chrome,
Safari and VSCodium. Compatibility depends on accessible editor text and caret
geometry; not every field or layout is supported. The app declares macOS 13+;
screen context requires macOS 14+. The supplied binary is arm64 only. Intel,
clean-machine/downloaded-app installation and actual logout/login startup remain
unverified. macOS may block the downloaded ad-hoc app; see the installation notes.

Personalization can infer names, spelling and sign-offs inconsistently. Docs has
limited layout coverage. Other unverified editor/terminal integrations remain
experimental. Broader ordinary-use feedback continues after release.

An ad-hoc update may require renewed Accessibility approval. The installer
preserves the previous app, but does not back up settings/history. Review data
compatibility before downgrading. Reload Docs tabs after reloading the companion.
