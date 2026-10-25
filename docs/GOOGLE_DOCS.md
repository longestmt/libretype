# Optional Google Docs companion

This integration is in development. Basic suggestion display and Shift-Tab
insertion have passed a native Google Docs check; full compatibility testing
is still in progress.

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
4. In Google Docs, choose Tools → Accessibility and enable screen reader and
   braille support. Keep any accessibility options you already use.
5. Focus an editable document and try a synthetic sentence first. Opening the
   extension popup reports whether the local companion is enabled.

Disable the setting in LibreType to stop using companion geometry. Removing the
Chrome extension also stops the connection. This does not change your Docs
accessibility settings, which may also be used by other assistive software.

## Data and boundaries

The extension sends viewport, editor and caret rectangles plus SHA-256
fingerprints of the document path and accessible text. It does not transmit the
text itself, edit the document, handle acceptance keys, use the clipboard, or
open a network service. Fingerprints are still derived from document data; they
are transient and are not included in application diagnostics or saved history.

Chrome starts a small Rust native-messaging host. Only the pinned extension ID is
allowed. The host forwards bounded messages through a private user-owned Unix
socket to LibreType; it does not use Accessibility itself. Companion messages
expire after 800 ms and are accepted only when the app setting is enabled.
LibreType additionally requires the foreground Chrome document, native text
fingerprint, native proxy caret and viewport scale to match. Existing secure-input,
website exclusion and focus checks remain in force. Unsupported geometry stays
unsupported; it does not enable a guessed caret or insertion fallback.

Initial development targets collapsed-caret suggestions and ordinary insertion.
Selected-text alternatives, document tables, complex layout, zoom and window
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
unzoomed. The next build scales that metadata using the paired visible/braille
caret geometry. Native validation of font scaling, longer suggestions requiring
multiple chunks, scrolling, and additional focus transitions remains pending.

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
