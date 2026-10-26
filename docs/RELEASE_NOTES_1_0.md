# LibreType 1.0 release candidate — build 66

Draft for a source-first developer release. Not yet published or promoted to
stable. Application version: 1.0.0; candidate build: 66; source revision:
`0bcb1b1bd924ef24dfa70e9d7f508d96d20a6d22`.

LibreType provides local writing suggestions on macOS through a separately
installed Ollama service. It has no subscription or account requirement.

## Included

- Inline suggestions with Tab word acceptance and Shift-Tab full acceptance.
- Mid-line previews, dismissal, manual activation and pause/resume.
- Writing instructions and exclusions scoped to apps and websites.
- Spelling, emoji shortcodes, alternatives, synonyms and accepted-word counts.
- Local model selection and optional clipboard/screen context.
- Optional experimental Google Docs companion and encrypted personalization.

## Artifacts

The complete-source archive is the primary download:
`libretype-1.0.0-candidate-build66-source.tar.gz`. It includes locked dependency
source, matching Rust runtime source, build tooling and notices. Use the
toolchain recorded in `RELEASE.json`; Ollama and model weights are separate.

The optional `LibreType-1.0.0-candidate-build66-macos-arm64.zip` contains the
Apple-silicon app. It is ad-hoc signed, **not Developer ID signed or notarized**.
macOS may block a downloaded copy; downloaded installation has not been verified.
See the installation notes before using it. No Apple Developer membership is
required to build this source using the supplied ad-hoc packaging script.

Verify downloads against `libretype-1.0.0-candidate-build66-SHA256SUMS.txt` from
the same trusted release. Application-owned source is GPL-3.0-only; third-party
materials retain their licenses. Both source and binary downloads must remain
available together if the binary is published.

## Verification and limits

The candidate's complete-source archive rebuilt and passed tests offline with
an empty Cargo cache. Rust, packaging and companion tests passed, as did notice
and ad-hoc signature verification. Rebuilding is not byte-for-byte reproducible.
The installed candidate passed the user-run native Chrome check for display,
word/full acceptance, dismissal, normal Tab navigation and pause/resume.

Compatibility depends on accessible editor text and caret geometry. Previously
tested basic flows include TextEdit, Signal, Gmail, Discord, Chrome, Safari and
VSCodium; support for every field or editor is not claimed. The app declares
macOS 13 or later, with screen context requiring macOS 14 or later. Intel and
clean-machine installation are unverified.

Personalization is experimental and off by default; inferred names, spelling and
sign-offs can be inconsistent. Google Docs is an optional experimental Chrome
integration with limited layout coverage; the final companion 0.1.9 live check
is still pending. Unverified terminal/editor integrations remain experimental.
Login-launch verification and ordinary-use feedback are also pending. These
notes must be updated with their actual outcomes before promoting the candidate
to stable.

Updates to ad-hoc builds can require renewed Accessibility approval. The
installer retains the prior app, but does not back up settings/history. See the
documented data-compatibility limits before downgrading.
