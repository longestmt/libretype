# Status and limitations

As of 2026-10-26, the current source passes its deterministic Rust test suites.
Optional live tests are ignored by default and need deliberate execution.

The 1.0 feature set is frozen. Release preparation takes priority over the broader
backlog below. Personalization and unverified integrations remain experimental;
their quality/coverage improvements are not 1.0 blockers. See the finite
[1.0 release checklist](RELEASE_1_0.md).

Build 66 (1.0 release candidate) is now installed. Its complete-source archive
passed offline rebuilding and tests, and its local preview and settings-label
layout passed. Accessibility and keyboard access are active. The user confirmed
build 66's final Chrome checks for display, word/full insertion, dismissal,
normal Tab navigation and pause/resume; live diagnostics subsequently reported
verified insertion. These results apply to the disposable editor tested.
Historical build numbers below describe when earlier checks were performed.

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
paused. The current source also cancels automatic analysis if keyboard activity
resumes, preserving the stored profile and the existing retry interval. Manual
analysis continues when typing resumes. Worker regression tests verify epoch
cancellation, rejection of a delayed canceled result, an unchanged encrypted
snapshot, and preservation of a subsequent manual build. A synthetic local-model
check also observed three canceled calls exit within 3–22 ms, with unchanged vault
bytes and successful following completions; see [limits](PERFORMANCE.md).
A later paired synthetic check found foreground request medians of 0.53 seconds
after cancellation versus 4.26 seconds with analysis continuing (0.41 seconds
without analysis), with the same completions and lower delay in all six pairs.
This change is installed in build 64; interruption through native keyboard input
and its full typing-to-draw impact remain unmeasured.
A synthetic lifecycle check with the local Gemma4 E4B model verified
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
A subsequent disposable Google Sheets check produced completions in three cells,
including mid-line insertion with the original suffix preserved. Build 44 reported
unconfirmed text/caret state after that test. An isolated mid-line check in build
45 displayed a preview, inserted five UTF-16 units, and verified the resulting
text and caret in the editor; the original suffix remained intact. This does not
establish the cause of the earlier warning or full spreadsheet compatibility.
A narrow-cell check in build 45 exposed automatic suggestions intercepting Tab.
Sheets uses a floating editor without an accessible cell ancestor. The current
source also recognizes its exact DOM identifier and A1-style cell address in a
known Chromium browser on the exact Sheets host. Compact recognized cells require
manual activation. The address stays local to the activation key and is not
included in prompts or diagnostics. Activation survives editor growth but is
revoked on cell-address, position or focus changes. Regressions pass. In build 46,
the user verified that Tab navigated out of the narrow cell without adding text,
then manual activation displayed a suggestion and Shift-Tab inserted it. The
cell's completed text was inspected and LibreType reported verified insertion.
The temporary transition recorder had ended before that interaction, so its
individual state changes were not captured. This verifies the reported workflow
in the disposable sheet, not every spreadsheet editor or cell configuration.

That check also exposed an on-screen, zero-width editor being classified as
visible. The current source rejects such editor bounds while preserving normal
zero-width insertion carets. Visibility regressions pass. Build 46's local preview
and native mid-line insertion were verified with Accessibility active.
Build 58 later verified suppression in disposable zero-width and zero-height
Chrome inputs, with suggestions returning in the visible control.
Unconfirmed-insertion diagnostics now distinguish unchanged text, a moved caret
with the expected full text, and differing prefix/suffix text. These labels
contain no writing; they do not relax verification or retry posted text.

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

A [synthetic resource measurement](PERFORMANCE.md) with the model already loaded
completed 12 production-pipeline requests with a 1.15-second median. During that
run build 47 used 36–40 MiB RSS and the Ollama model process used 3.78–3.97 GiB RSS.
The repeatable sampler records process-level data and initial/final model state;
it does not establish unique physical memory, battery impact or native typing
latency. No application settings were changed.

A separate [synthetic writing check](MODEL_QUALITY.md) compared 16 contexts across
three local models, with two rounds each. It identified missing-word and factual
specificity weaknesses in the smaller Qwen variants; a grammar-prompt candidate
did not improve the observed gaps and was not adopted. Qwen3 4B was downloaded
and verified through the production worker. The selected model stayed unchanged.
An additional synthetic personalization check found that learned profiles can
omit regional spelling preferences and that profile influence can defeat an
explicit spelling override in a tested context. Two prompt candidates were
rejected; the reproducible probe and evidence are described in the same document.

An unsent synthetic draft in VSCodium's Twinny chat exposed a separate routing
gap: its webview composer was classified as an unknown code-editor field and
required manual activation. Build 47 recognizes the focused ProseMirror composer
only with a specific composer ancestor and the nearest web area's exact Twinny
extension identity. Model settings, transcript editors, unrelated origins and
similarly named classes are rejected by regression checks. The identifiers came
from a read-only packaged-interface inspection; no extension implementation was
copied. A text-free diagnostic distinguishes source, chat, extension chat and
other controls. With build 47 installed and Accessibility renewed, the native
trace classified the composer as extension chat and displayed ghost text. The
user accepted it with Tab; LibreType verified eight UTF-16 units in one chunk,
and the completed unsent draft was inspected. The nearby model-name field showed
no suggestion, and returning to the composer restored ghost text. That field was
rejected by the earlier security-metadata gate, so the native check does not
independently exercise its classification as Other. The result verifies this
installed Twinny composer, not every editor extension or version. Settings and
privacy controls were unchanged, and no chat message was submitted.

Build 49 replaced the two dependencies whose exact license notices were missing.
Its in-memory OCR check, Foundation geometry round trips and independent Clang
callback lifetime tests passed. After Accessibility renewal, the user verified
native ghost display and Tab acceptance in the unsent Twinny test draft; the
app confirmed eight UTF-16 units inserted in one chunk. Settings were unchanged.
This does not verify live screen capture. The matching source archive rebuilt
with an empty Cargo cache and Cargo offline mode; all-target tests and the notice
inventory check passed. Dynamic library paths refer only to macOS system files.

Build 60 verified optional live screen context in an isolated Chrome window.
The earlier selector assumed the first normal window in the app's window list
owned the caret and never reached capture in this layout. The updated selector
matches the focused editor's Accessibility window bounds to a unique valid
on-screen window, checks that it contains the caret, and revalidates the window
after capture. Ambiguous matches remain rejected. Regression checks cover
auxiliary windows, wrong owners, changed bounds, hidden windows and ambiguity.

The authorized native check recorded successful capture and OCR, displayed the
reference time 14:20, and verified insertion of “14:20 on Wednesday” in one chunk.
The user confirmed the result, and the final synthetic draft was inspected.
A separate saved-image check also recognized the pictured time and used it in
three completion requests. These checks establish the tested window and layout,
not universal OCR accuracy or compatibility. Screen context was turned off and
all original settings restored; clipboard context and history remained off.

## Broader backlog and release evidence

Items 1–2 and expanded performance experiments are post-1.0 work. Final package,
startup, acceptance and distribution checks are tracked separately in the
[1.0 checklist](RELEASE_1_0.md).

1. **Personalization quality.** A source fix now selects distinct eligible
   excerpts before spending the sample budget. Repeated recent records and short
   legacy records previously hid usable older samples and could prevent profile
   generation. Regression and all-target tests pass; this fix is installed in build 66.
   It does not resolve model extraction or instruction-priority failures.
   The native storage/profile lifecycle works, but
   inferred spelling conventions and explicit-instruction priority have known
   failures. A source fix now preserves punctuation in model-selected recurring
   greetings and sign-offs, which previously dropped valid entries such as
   `Cheers!`. Source-support and encrypted-round-trip tests pass; this fix is
   included in installed build 64 and does not resolve model extraction or preference quality.
   An explicit-preference reminder improved one Gemma spelling conflict
   and one Qwen sign-off conflict while preserving compatible-name controls.
   Removing a conflicting blanket vocabulary-spelling directive also corrected
   the earlier neighbour/neighbor case on Gemma. Twenty additional paired cases
   remained unchanged, including unresolved spelling and sign-off failures with
   a longer profile. The change is included in build 61.
   Two deterministic spelling-evidence candidates improved extraction but
   regressed explicit overrides, so neither was adopted. Keep frozen cases and
   compatible-profile controls when evaluating fixes; suppressing all
   personalization would not close this gap.
   A separate system-message experiment also failed: one variant made no target
   improvement, and another recovered an identifier but regressed two previously
   passing spelling/context checks. Neither was adopted. A candidate must then
   be checked on new writing contexts before recommending it.
   A newer small-model follow-up did not improve the preservation gate. A local
   MLX 12B model improved some longer-profile conflicts but remained inconsistent
   and used substantially more memory. Optional MLX compatibility is implemented
   and passed synthetic completion, synonyms and profile checks; native editor
   display and Shift-Tab insertion were user-confirmed in build 62's requested
   unsent Gmail check. The original model and settings were restored. There was
   no contemporaneous acceptance trace, so this is not machine-verified edit
   integrity or a broader editor-compatibility claim.
   A subsequent recheck of the currently installed `gemma4:e4b` alias found a
   different MLX artifact. Its shorter frozen checks passed, but longer-profile
   spelling and sign-off failures remained. A targeted spelling-precedence
   reminder changed none of nineteen outputs and was reverted. Model identity
   and evaluation limits are recorded in [model quality](MODEL_QUALITY.md).
   A separate chat-role candidate corrected one toolkit choice but regressed a
   previously preserved verbatim sign-off across 43 paired contexts. It was
   rejected and reverted; the existing generation path remains unchanged.
2. **Editor coverage.** Extend code-editor policy checks beyond the verified
   VSCodium source editor and Twinny composer to other chat/source surfaces,
   and verify terminal AI prompt/multiplexer layouts. The reported spreadsheet
   workflow is also verified; none of these checks establishes universal editor
   support. Orca has no verified dedicated integration. Its inspected
   [`terminal read` contract](https://github.com/stablyai/orca/blob/fd26aef37c5f512eb75cd36828ae49cb1b7e0bc6/src/shared/runtime-terminal-contracts.ts)
   exposes terminal text, not a typing-caret offset or pane screen rectangle;
   its paging cursor must not be treated as an editor caret. A live focused-pane
   Accessibility check remains necessary before choosing an adapter.
   Build 58's disposable
   Chrome check passed for zero-width and zero-height inputs: the user reported
   suggestions only in the visible control, and native diagnostics recorded
   suppression followed by a visible suggestion after returning to that control.
   This establishes those two synthetic layouts, not every hidden-editor case.
3. **Performance and setup.** Model discovery now uses one background worker,
   coalesces refresh requests, and retries when Ollama is unavailable. Healthy
   inventories refresh once a minute. Local HTTP integration tests verify recovery,
   local-only model filtering, malformed replies, response-size and whole-request
   limits; scheduler tests cover backoff, concurrent refresh requests and worker
   failure. The build 61 check verified Accessibility and keyboard shortcuts;
   its live model catalog was available, and settings were unchanged. Native
   suggestion display and Shift-Tab insertion passed the user’s unsent-draft
   check. The user did not identify which of the offered apps was tested. These
   checks do not establish actual login launch or a clean-machine installation.
   Build 58 captured three native typing-to-draw
   samples in Chrome: 7.50, 2.08 and 2.10 seconds, dominated by generation.
   See [measurement limits](PERFORMANCE.md). Extend this to controlled workloads
   and combined application/model resource use, then check actual login launch
   and clean-machine setup. These samples do not establish broad latency,
   battery impact or a complete setup experience.
4. **Release delivery.** The dependency replacements remove the two missing-notice
   issues, and build 49 passed the native insertion check. The build 61 source
   export rebuilt and passed tests offline with an empty Cargo cache; the build
   62 export subsequently passed the same validation. Builds 63 and 64 were
   prepared, and their matching sources also rebuilt and passed tests offline;
   build 64 is now installed, while build 63 was superseded without installation.
   Build 64 consolidates profile punctuation, automatic-analysis interruption and
   companion 0.1.7. Its Accessibility and keyboard listener checks are active;
   its native editor smoke check remains pending. All four rebuilt bundles
   retained the notices and verified their ad-hoc signatures. This does not
   establish bit-for-bit reproducibility or public-release readiness. Complete contributor review
   documented in [release instructions](RELEASING.md), then verify signing,
   upgrade Accessibility continuity, notarization and distribution. These are
   release gates separate from editor feature implementation.

Field-adjacent clickable controls are not implemented. Related actions are
available through shortcuts and settings; previews remain non-interactive in
accordance with the chosen interaction design.

Google Docs has experimental support through an [optional Chrome companion](GOOGLE_DOCS.md),
off by default. Native checks passed for basic and mid-line insertion, stable
word/full acceptance, 150% document zoom, and long-document scrolling and
insertion. Companion 0.1.6 also completed its live accessibility-helper save flow,
with continued suggestions confirmed by the user. The tested document already
supported suggestions. A later initially-disabled check stopped at the Tools
menu. After the pointer-position correction in companion 0.1.7, a live check
enabled screen reader and braille support from Off and verified both after a
page reload. The other test-state checkbox values were preserved; the original
collaborator-announcement preference was restored afterward. An already-enabled
follow-up returned a review result and is not counted as a pass. Companion 0.1.8
adds fixed, text-free interruption and step-timeout reasons. Version 0.1.9 also
fixes cancellation caused by unrelated background-tab navigation or closure;
39 synthetic tests pass, including protection of the active document during
overlapping scope lookups. Live verification of these changes is pending. Browser-restart
persistence remains unverified. Selected-text alternatives, emoji/spelling replacements, tables,
browser zoom and complex layouts are not covered by this support claim.
Cross-Mac synchronization is outside the project scope.

Local development evidence is retained outside the public source manifest.
Passing a narrow synthetic check is not treated as proof of universal support.
