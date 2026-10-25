# LibreType

A macOS typing assistant written in Rust, with local suggestions through a
separately installed Ollama service. No subscription, account or usage quota.

## Features

- Inline ghost text, with word-by-word or full acceptance and a stable remainder.
- Mid-line suggestions in a small, non-interactive preview above the caret.
- Global, app and website writing instructions and exclusions.
- Custom shortcuts, manual activation, pause/resume and dismissal controls.
- Spelling suggestions, emoji shortcodes, word alternatives and synonyms.
- Local model selection and explicit downloads.
- Optional clipboard/screen context and encrypted writing-history personalization.
- Accepted-word statistics and optional launch at login.

Compatibility depends on the editor exposing accessible text and a usable caret.
Basic flows have been exercised in TextEdit, Signal, Gmail, Discord, Chrome,
Safari and VSCodium; this is not a promise of support for every field in those apps.
Google Docs has experimental support through an [optional Chrome companion](docs/GOOGLE_DOCS.md).
See [status and limitations](docs/STATUS.md).

## Build and run

Requirements: macOS, Xcode command-line tools, Rust/Cargo, Python 3.11+, and
Ollama with a local model. The application uses Apple frameworks and system libcurl.

```sh
cargo test --locked --all-targets
./scripts/build-app.sh
python3 scripts/app_bundle.py check
python3 scripts/app_bundle.py install --quit --allow-identity-change
```

The build creates `dist/LibreType.app`; installation defaults to
`~/Applications/LibreType.app`. Development builds are ad-hoc signed and replacing
them can require removing and re-adding the installed app in macOS
**Privacy & Security → Accessibility**. The installer preserves a rollback copy.
If the renewed approval does not take effect, quit and reopen LibreType before
testing suggestions again.
Certificate builds can use `--identity` with a valid signing identity; notarization
and public release hosting are not configured.

Keep Ollama running, open LibreType, select a local model and grant Accessibility
for suggestions in other apps. The Settings preview works without that permission.
The LT menu provides settings, pause/resume and model selection.

## Default controls

| Action | Shortcut |
| --- | --- |
| Accept next word | Tab |
| Accept full visible suggestion | Shift-Tab |
| Dismiss | Esc |
| Request suggestion | Control-Option-Space |
| Pass Tab through while a suggestion is visible | Option-Tab |

Typing cancels stale suggestions. Tab retains normal behavior without an actionable
suggestion. Shortcuts and dismissal behavior are configurable. Suggestions use the
editor's exposed font where available, with a caret-based fallback.

Compact editors identified by Accessibility as table cells wait for manual
activation, preserving Tab navigation. Control-Option-Space enables suggestions
in the current cell temporarily. Ordinary small fields and larger cell editors
retain automatic suggestions. Chrome's floating Google Sheets editor is also
recognized by its exact identifier, cell address and document host. Narrow-cell
navigation and manual insertion have been checked in a disposable sheet, along
with mid-line insertion. Other cell configurations and spreadsheet apps may differ.

## Privacy and local storage

Inference uses a numeric loopback connection to Ollama. Downloads require internet.
No model weights or Ollama executable are included in the application bundle.
Models have [separate licenses](docs/MODELS.md).

Clipboard context, screen context, history recording and learned-style influence
start off. History additionally requires explicit unlock each app launch; its
key is stored in macOS Keychain and records are encrypted locally. Screen context
requires the relevant macOS permission. Secure fields and excluded contexts are
not eligible. Optional data sources should be enabled only when wanted.

The bundled executable and process are named `LibreType`; the Cargo command-line
binary is `libretype`. Legacy storage identifiers remain for existing installations:
the bundle ID is `org.libretypist.app`, settings/history are under
`~/Library/Application Support/LibreTypist`, and health diagnostics are under
`~/Library/Caches/LibreTypist`. Health snapshots exclude typed text and completions.
Do not publish those local directories.

## License and releases

LibreType's application-owned source and documentation are **GPL-3.0-only**.
You may redistribute and modify them under GPL version 3. The program comes
without warranty. See [LICENSE](LICENSE), [COPYRIGHT](COPYRIGHT), and
[third-party notices](THIRD_PARTY_NOTICES.md). Third-party materials retain their
own licenses; model licenses do not become the application's license.

[Release instructions](docs/RELEASING.md) cover matching source delivery, notices,
and outstanding release gates. No public release or source URL is configured yet.
The repository intentionally excludes local research, backups, captures, compiled
apps, model files and generated release archives. Required source/build inputs
must remain in the reviewed public file list.
