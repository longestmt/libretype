# Status and limitations

As of 2026-10-23, the current source passes 221 deterministic Rust tests.
Optional live tests are ignored by default and need deliberate execution.

Verified workflows include basic suggestion display/acceptance in the tested
apps, browser mid-line insertion, spelling and emoji replacement, alternatives,
scoped instructions, clipboard context, daily counts and visibility changes.
TextEdit history capture now preserves a manually typed paragraph, waits for
verified acceptance, survives lock/unlock and supports scoped deletion. All six
personalization strength levels save through the native controls; Strong was
verified after a process restart, then the original Off setting was restored.
That test kept history locked and recording disabled.

Automatic profile refresh now waits for five seconds without keyboard activity
even when recording is off, and does not start while suggestions are globally
paused. A synthetic lifecycle check with the local Gemma4 E4B model verified
three encrypted samples, scheduled profile generation, lock/unlock persistence
and a completion using the restored profile with recording disabled. This used
a temporary vault without Keychain or real writing; it does not verify native
editor attachment or prove that the learned style improves completion quality.

A native Chrome paste check found that an empty field without caret geometry
lost its initial history baseline. The current source keeps a separately typed,
verified empty baseline for history while continuing to require real caret
geometry for suggestions and insertion. It also uses a non-interactive preview
when an end-of-text completion cannot fit at the right edge. Regression tests
pass. In build 42, a native Chrome check verified the right-edge preview and
insertion, and manual pastes into initially empty fields produced encrypted
history records. Duplicate samples were correctly rejected as insufficient
distinct data. After three distinct synthetic samples were available, automatic
refresh built a profile with recording off. A production Chrome completion
request attached that profile and displayed ghost text. Setting influence to
Off produced another visible suggestion without attaching the loaded profile.
Scoped deletion removed the test records and profile; original preferences
were restored and history was locked. This verifies the native lifecycle, not
comparative style quality or support in every editor.

Compact editors with an accessible table-cell ancestor now require manual
activation. The activation is restricted to the current cell, expires after
30 seconds, and is revoked on focus loss, cancellation or a cell-address change.
Ordinary small fields and larger cell editors remain automatic. Geometry and
activation-boundary regressions pass. A native Chrome check in build 44 observed
compact-cell inactivity, manual activation and verified insertion, plus automatic
suggestions in the larger cell editor. The user confirmed the requested checks.
This does not yet verify Google Sheets or other spreadsheet apps.

The native cell test exposed an earlier rejection of ordinary single-line Chrome
inputs with no subrole. The updated source permits this only for known Chromium
browsers that explicitly report no subrole value and expose an unprotected
visible-character range. Secure roles and failed/unsupported metadata stay
rejected. The native check captured ordinary single-line fields and inserted a
completion into a compact input; focusing the synthetic password control activated
secure input and suppressed suggestions. The secure-subrole rejection path also
has deterministic regression coverage.

A 30-second passive observation of build 44 on an Apple M1 measured about
29 MiB of resident memory and 1.2% of one CPU core for the LibreType process.
Terminal was foreground, capture reported an unavailable text window, and no
suggestion was active. Settings were unchanged. This is a narrow baseline,
not a controlled idle benchmark: it excludes Ollama, GPU and accessibility-server
work, and does not establish peak memory, typing latency or battery use.

A separate [synthetic writing check](MODEL_QUALITY.md) compared 16 contexts across
three local models, with two rounds each. It identified missing-word and factual
specificity weaknesses in the smaller Qwen variants; a grammar-prompt candidate
did not improve the observed gaps and was not adopted. Qwen3 4B was downloaded
and verified through the production worker. The selected model stayed unchanged.

Still requiring broader validation:

- Learned-style quality and profile behavior across more editors and models.
- More editor/focus/selection combinations, code-editor chat and source policies.
- Live screen OCR context; it remains optional and off by default.
- Terminal AI prompt adapters and multiplexer layouts beyond synthetic tests.
- Consistent instruction compliance, grammar, latency, memory and battery use.
- Actual login launch, clean-machine setup and additional model downloads.
- Signed-upgrade Accessibility continuity, notarization and release delivery.

Google Docs support is deferred; any future companion must be optional.
Cross-Mac synchronization is outside the project scope.

Local development evidence is retained outside the public source manifest.
Passing a narrow synthetic check is not treated as proof of universal support.
