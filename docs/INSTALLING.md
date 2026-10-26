# Installing the 1.0 candidate

Build 66 is a local release candidate, not a published stable release. Its binary
is for Apple silicon (`arm64`). The app declares macOS 13 or later; optional screen
context requires macOS 14 or later. This declaration is not verification on every
supported OS version. Intel builds and clean-machine installation have not been
verified for this candidate.

## First run

1. Install and start Ollama separately. LibreType includes neither its service nor
   model weights. Select a local completion model; model downloads are explicit
   and have their own license notices.
2. Keep one working copy of `LibreType.app` at `~/Applications/LibreType.app`.
   Open that copy. The app lives in the **LT** menu bar; it does not require a Dock
   icon. Development installation and rollback commands are in the
   [README](../README.md#build-and-run).
3. In **LT → Local models**, select an installed model. If the service is
   unavailable, start Ollama; LibreType retries discovery. A missing model and
   an unavailable service are reported separately.
4. Grant this copy of LibreType access in macOS **Privacy & Security →
   Accessibility**. An ad-hoc signed update can require removing and re-adding
   the app there. Quit and reopen LibreType if approval has not taken effect.
5. Use **Settings & preview** to try a completion. Then type `Thank you for your `
   (including the final space) in an unsent draft in a supported editor. Tab
   accepts a word, Shift-Tab accepts the full suggestion, and Esc dismisses it.
   Local generation can take several seconds, especially on a cold model.

History, learned influence, clipboard context and screen context start off.
They are not required for basic suggestions. History additionally requires an
explicit unlock each launch. Google Docs requires a separately enabled optional
[Chrome companion](GOOGLE_DOCS.md); it is experimental.

## Startup and common checks

**LT → Startup…** reports macOS login registration and offers **Open LibreType
at login**. Enabling it does not start Ollama. macOS may require Login Items
approval. Actual logout/login behavior has not yet been verified for build 66;
opening the app normally is not evidence that login launch works.

If suggestions are absent, check that suggestions are not paused, the local model
is available, Accessibility is active, and the app/site has not been excluded.
Secure fields deliberately receive no suggestions. Compatibility depends on the
editor exposing text and a usable caret; see [tested workflows](STATUS.md).
Compact recognized spreadsheet cells require Control-Option-Space for activation
so Tab can continue to navigate cells.

The candidate is ad-hoc signed, not Developer ID signed or notarized. General
download installation is still a distribution gate; these instructions do not
claim Gatekeeper approval for a downloaded copy. Keep the corresponding source
and checksum files with any approved distribution, following
[release instructions](RELEASING.md).
