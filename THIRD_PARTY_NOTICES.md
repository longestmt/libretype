# Third-party notices

LibreType's application license is GPL-3.0-only. This does not relicense third-party
code or data. Preserve their copyright, permission and attribution notices.

- [Exact dependency inventory](third_party/notices/inventory.json): package
  versions, declared terms, authors, repositories, runtime/build reachability,
  notice paths and hashes; generated from Cargo.lock for the specified target.
- [Collected license texts](third_party/notices/): includes nested component
  notices and the exact compiler's Rust standard-library copyright report.
- [gemoji](vendor/gemoji/SOURCE.md): unchanged emoji JSON, licensed under
  [MIT](vendor/gemoji/LICENSE). Emoji glyphs are rendered by macOS.
- [curl bindings](vendor/curl/SOURCE.md): original [MIT notice](vendor/curl/LICENSE).
- [Patched Objective-C bindings](vendor/objc/SOURCE.md): original MIT source with
  a GPL-3.0-only ownership adapter and raw-pointer fixes. The adapter uses
  `malloced` 1.3.1; its MIT and Apache-2.0 notices are retained in the inventory.
- [Separate model licenses](docs/MODELS.md): no model weights or Ollama binary
  are included in LibreType's application bundle or source release.
- [Open review items](third_party/REVIEW.md): contributor-authority review
  and release delivery checks remain; the current inventory has no missing notices.

Dependency declarations include MIT, Apache-2.0, ISC, Unicode-3.0 and compatible
alternative-license expressions. Retaining both alternatives is not a requirement
to elect both; combined AND obligations (including ring and Unicode data) remain.
The inventory includes build-time tools for completeness and does not assert
that every listed package is embedded in the binary. System libraries are
provided by macOS, not copied into the app.
