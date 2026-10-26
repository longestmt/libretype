# Optional Google Docs companion

This experimental integration supports collapsed-caret suggestions and insertion
in Chrome. Native checks passed for word/full acceptance, mid-line insertion,
document zoom and long-document scrolling. Broader compatibility testing is
still in progress.

Google Docs draws a visible caret separately from its off-screen braille editing
surface. The optional Chrome extension supplies geometry to the Rust application;
LibreType continues to obtain text and selection through macOS Accessibility and
to generate and accept suggestions through its existing local pipeline.

## Enable after installing a compatible build

1. Open LibreType and enable “Google Docs companion” in Settings, then Save.
2. Inspect the local registration with `python3 scripts/install_docs_companion.py`.
   Run it with `--install` to register the bundled native host for Chrome.
3. In `chrome://extensions`, enable Developer mode, choose Load unpacked, and
   select the directory printed by the registration command. This development
   extension requests access only to `https://docs.google.com/document/*` and
   permission to communicate with its specific local native host.
4. With companion 0.1.6 or later, open its toolbar popup while a Google Doc is active and
   click **Enable Docs accessibility**. The popup closes; leave the document
   focused while setup runs, then reopen the popup to see **Last setup**.
   Alternatively, choose Tools → Accessibility in Docs and enable screen reader
   and braille support manually. Keep any accessibility options you already use.
5. Focus an editable document and try a synthetic sentence first. Opening the
   extension popup reports whether the local companion is enabled.

Disable the setting in LibreType to stop using companion geometry. Removing the
Chrome extension also stops the connection. This does not change your Docs
accessibility settings, which may also be used by other assistive software.


## Accessibility setup helper

The companion provides an **Enable Docs accessibility** button in its popup.
The button requires the local companion to be enabled and an active editable
Google Doc. It opens Tools → Accessibility, enables screen reader and braille
support only if needed, verifies that the other checkbox states have not changed,
and saves. It does not run automatically, edit document content, send document
text, or request additional extension permissions.

Chrome can report the document window as unfocused while the action popup is
open. The helper therefore closes the popup first and waits up to three seconds
for the same active document in the same window to regain focus. Tab changes,
navigation, another Chrome window, or a timeout stop setup. Existing focus and
opt-out checks remain active during the operation. Reopening the popup shows the
last setup result, held only in worker memory with no document address or text.

The helper recognizes English menu and checkbox labels. Unrecognized controls,
a pre-existing dialog, a focus/navigation change, or an unrelated option change
stop the operation. If it stops after opening the dialog, review or cancel that
dialog before retrying; partially changed controls are not automatically saved.
The popup retains manual instructions for unsupported layouts or languages.

The 29 extension tests and the packaging checks pass. Synthetic browser checks
verified enabling missing options, leaving already-enabled options alone, and
stopping without saving after interruption or unrelated preference changes.
A live 0.1.6 check completed the save flow: the user supplied a screenshot showing
**Last setup: Settings saved** and confirmed that suggestions worked afterward.
The tested document already supported suggestions before this check, so that
result alone did not establish initial enablement.

An initially-disabled check later stopped at the Tools menu without opening the
Accessibility dialog, including an uninterrupted retry. The original settings
were restored and verified. Version 0.1.7 sends pointer events at each control's
actual position instead of the default `(0, 0)`, and refuses off-screen controls.
A pointer-sensitive menu regression fails before this change and passes after;
a real Chrome fixture also verifies native checkbox activation and preservation
of unrelated options. After the user reloaded 0.1.7, a live check started with
screen reader and braille support both disabled. The helper reported **Settings
saved**, and reopening Accessibility settings confirmed both were enabled.
Collaborator announcements and magnification remained off, matching the test's
starting state. Both required options remained enabled after reloading the page.
The original collaborator-announcement preference was then restored to On;
magnification remains Off, and both required options remain On.

An additional already-enabled attempt returned **Setup stopped**. Its final
checkbox states were unchanged; a later native observation showed Chrome on a
different page, but the exact interruption stage was not recorded. This attempt
is not counted as successful live idempotence verification. Browser-restart
persistence and other document layouts remain unverified.

The app-bundled companion in build 62 remains 0.1.3. The standalone 0.1.7
extension can be loaded independently without replacing or re-signing the app.
Future bundles built from this source include 0.1.7.

## Data and boundaries

The extension sends viewport, editor and caret rectangles, native and value-string
UTF-16 caret offsets,
and SHA-256 fingerprints of the document path and accessible text. It does not transmit the
text itself, edit the document, handle acceptance keys, use the clipboard, or
open a network service. Fingerprints are still derived from document data; they
are transient and are not included in application diagnostics or saved history.

Chrome starts a small Rust native-messaging host. Only the pinned extension ID is
allowed. The host forwards bounded messages through a private user-owned Unix
socket to LibreType; it does not use Accessibility itself. Companion messages
expire after 800 ms and are accepted only when the app setting is enabled.
LibreType additionally requires the foreground Chrome document, native text
fingerprint, exact caret offset, hidden editing surface and viewport scale to
match. Existing secure-input,
website exclusion and focus checks remain in force. Unsupported geometry stays
unsupported; it does not enable a guessed caret or insertion fallback.

Initial development targets collapsed-caret suggestions and ordinary insertion.
Selected-text alternatives and range replacements (including emoji and spelling
fixes) are unavailable in the Docs companion. Its adapter currently verifies
collapsed carets only. Document tables, complex layout, browser zoom and window
changes require explicit compatibility checks before broader support is claimed.

## Development validation

Build 52 matched the visible Docs caret with native text and selection, generated
a suggestion locally, and inserted it with Shift-Tab in a disposable document.
The post-insertion check exposed a delay between the edit and the companion's
new text fingerprint. Build 53 adds a bounded wait for that geometry
refresh, restricted to an unchanged focused Docs editor. It never reposts a
chunk. A native check subsequently reported “Insertion verified in editor,” and
the user confirmed Tab followed by Shift-Tab inserted the suggestion correctly
without changing its remaining words. Mid-line insertion also preserved the
existing suffix and was verified natively. The Off control stopped companion
messages and hid suggestions; it was re-enabled after the check.

At 150% document zoom, caret mapping worked but the reported font size remained
unzoomed. Build 54 scales that metadata using paired visible/braille caret
geometry. Native checks showed approximately 15 points at 100% and 22.5 points
at 150%; the document was restored to 100% afterward.

A 60-paragraph scrolling check exposed three different representations of the
same 18-paragraph accessible window: native AXValue has 1,037 UTF-16 units,
DOM innerText has 1,054, and textContent/native caret positions have 1,020.
The native value contains one newline per paragraph boundary; native caret
positions omit those separators. Companion 0.1.3 reconstructs the native value
for fingerprint matching and supplies both caret offsets. LibreType retains
the raw offset for Accessibility calls and uses the verified value offset for
completion and history context. Invalid UTF-16 splits are rejected. Exact hidden
caret coordinates must still match, including empty paragraphs with equal raw
offsets. Regression tests cover this observed window and emoji/mid-line offsets.

Build 57 matched the 61-paragraph native window (3,471 raw caret units versus
3,531 value units) and displayed a suggestion at the end of the long document.
Scrolling the caret off-screen hid the overlay; scrolling back restored it with
a fresh native text/caret match. Shift-Tab then inserted a 35-unit completion
in two verified chunks, confirmed by the user and the final document text.
The native paragraph window shrank from 61 paragraphs to 18 during insertion
and still matched correctly. The disposable document was restored at 100% zoom.
Browser zoom and additional focus transitions still require native validation.

Run `cargo test --locked --offline`,
`node --test tests/docs_companion.test.js`, and the Python packaging tests.
Browser-side tests exercise message routing, permission scope and payload
validation with synthetic Chrome APIs. A disposable DOM fixture also exercises
the shipped content adapter: disabled/focus checks, text fingerprints, selection
and clipped-caret rejection, changes during asynchronous hashing, and clearing
on focus loss or page exit. These tests do not substitute for loading the
extension and testing its DOM adapter in a real Docs document.

Protocol references:
[Chrome native messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging),
[isolated content scripts](https://developer.chrome.com/docs/extensions/develop/concepts/content-scripts),
and [Docs accessibility setup](https://support.google.com/docs/answer/6282736?hl=en).
