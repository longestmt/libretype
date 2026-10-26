# Installing LibreType 1.0

The 1.0 release binary
is for Apple silicon (`arm64`). The app declares macOS 13 or later; optional screen
context requires macOS 14 or later. This declaration is not verification on every
supported OS version. Intel builds and clean-machine installation have not been
verified for this release.

## First run

Download the [LibreType 1.0 DMG](https://github.com/longestmt/libretype/releases/download/v1.0.0/LibreType-1.0.0-macos-arm64.dmg),
open it, and drag **LibreType.app** onto the **Applications** shortcut. Eject the
disk image before opening the installed app. A [ZIP download](https://github.com/longestmt/libretype/releases/download/v1.0.0/LibreType-1.0.0-macos-arm64.zip)
is also available; extract it and move the app to Applications yourself.

For updates, quit LibreType first. If your existing copy is in `~/Applications`,
replace it there instead of creating another copy in `/Applications`. Keep one
installed copy. Finder replacement does not create the development installer's
rollback backup. Both downloads contain the same build 67 app.

1. Install and start Ollama separately. LibreType includes neither its service nor
   model weights. Select a local completion model; model downloads are explicit
   and have their own license notices.
2. Open your installed `LibreType.app` from Applications. The app lives in the
   **LT** menu bar; it does not require a Dock
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

## Source/developer distribution

The release is source-first; an Apple Developer membership is not a
prerequisite for building the supplied source with the existing ad-hoc build
script. Follow [the complete-source build instructions](RELEASING.md#build-from-a-release-source-archive)
using the compiler version recorded in the archive's `RELEASE.json`.

The optional prebuilt app is ad-hoc signed, not Developer ID signed or
notarized. macOS may block a downloaded copy. If you independently trust its
source and integrity, Apple's [app-opening guidance](https://support.apple.com/en-us/102445)
describes the per-app **Privacy & Security → Open Anyway** flow. Availability can
depend on the warning and device policy. This release has not been tested
through that downloaded-app flow; successful local development launch is not
Gatekeeper approval. A checksum detects changed bytes but does not authenticate
an untrusted download source.

Keep the corresponding source and checksum files with any approved distribution,
following [release instructions](RELEASING.md). Developer ID signing and
notarization are a separate possible future distribution improvement, rather
than a prerequisite for publishing source. Apple's
[Developer ID documentation](https://developer.apple.com/developer-id/) describes
the program membership required for that signing route.
