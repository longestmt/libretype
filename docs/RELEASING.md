# Source and binary releases

Application-owned code and documentation use GPL-3.0-only. The full license is
in LICENSE. Third-party files remain under their own terms. This policy does not
withdraw MIT permissions already granted for earlier copies.

## Before publishing

1. Review `python3 scripts/public_source.py --list`. New file categories are
   excluded until explicitly admitted by `.gitignore`. Never force-add local
   research, app backups, credentials, captured writing or model weights.
2. Run `python3 scripts/license_inventory.py` with the release toolchain. It
   inventories the target's exact lockfile dependencies, distinguishes runtime
   reachability from build/procedural-macro reachability, and retains upstream
   and nested notices. Reachability is not a claim that every component survives
   linking. Inspect changed terms and any reported issues.
3. Resolve the open items in `third_party/REVIEW.md`. A notice inventory is not a
   substitute for rights to distribute. Confirm contributor authority and keep
   factual provenance records. Do not claim strict clean-room development.
4. Run Rust and packaging tests from a clean public-source export. Verify GPL,
   copyright, dependency and Rust runtime notices in the built bundle. Notices
   must be present before signing. Do not edit a signed bundle afterward.
5. Create a matching source archive with `python3 scripts/release_source.py`.
   It includes the reviewed application tree, locked Cargo dependency source and
   source supplied by the exact Rust toolchain for its runtime. The normal
   command refuses unresolved notice issues. A developer-only override exists
   for testing the export and must not be used to bypass release review.
6. Publish the matching source archive beside every binary, with equivalent
   access, a clear source download link, release tag/build number and SHA-256
   hashes. A GitHub source snapshot containing Cargo.lock alone is insufficient
   for the chosen complete-source delivery approach. Retain matching releases.
7. Verify the published downloads and source build. Supply installation
   information if GPLv3 section 6 requires it for the distribution arrangement.
   Do not impose extra application restrictions through a model agreement.

Development source is maintained in the private GitHub repository
`longestmt/libretype`. No public source URL or binary publication is configured.
No public release is authorized merely by running a build. Signing and
notarization are separate distribution tasks.

## Build from a release source archive

Use the Rust compiler version recorded in `RELEASE.json`, macOS SDK/command-line
tools, Python 3.11+ and Git. Extract the archive, then:

```sh
cargo build --offline --locked --release
cargo test --offline --locked --all-targets
python3 -m unittest discover -s scripts -p 'test_*.py' -v
```

`.cargo/config.toml` redirects dependencies to `third_party/cargo-src`. Source
for the toolchain runtime is retained under `third_party/rust-runtime-source`;
normal builds use the matching toolchain's installed standard library. The
compiler and Apple system libraries are separate system/build prerequisites.
Model weights and Ollama are installed separately and are not release contents.

## References

- https://www.gnu.org/licenses/gpl-3.0.html (especially sections 1, 4–6 and 10)
- https://www.gnu.org/licenses/gpl-howto.en.html
- https://www.gnu.org/licenses/gpl-faq.en.html

This is a technical release procedure, not a determination of authorship,
contractual obligations or legal clearance.
