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
   for testing the export and must not be used to bypass release review. The
   archive reports `notice_inventory_complete`, not public-release approval;
   contributor authority and delivery review remain separate.
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


## Build 61 source validation

The local archive `libretype-build61-source.tar.gz`, exported from source revision
`4d593c3af1dd72d404ea363e4833bfa6d0f27156`, was extracted to a temporary directory
and rebuilt with an empty Cargo cache and Cargo offline mode. Its 12,844 manifest
file hashes matched. Compilation inputs were unchanged from installed build 61;
intervening changes only updated status and review documentation.

The offline release build, all-target Rust tests (200 library and 241 application
tests, with overlap), 18 packaging tests, 14 companion tests and notice inventory
check passed. The exported tree also produced an app bundle with byte-checked
dependency/runtime notices and a valid ad-hoc signature. Dependency and runtime
source directories stayed in the source archive and were not copied into the
binary bundle. The temporary build was removed after validation.

Archive SHA-256:
`ec8725d9d0276294536fa111371ee6ce04b11f4d1a9961d5628989fd6c5c24b2`.

The rebuilt signed executables were not byte-identical to the installed ones;
this check establishes offline buildability, not bit-for-bit reproducibility.
It does not replace native startup checks, contributor review, certificate
signing, notarization or verification of published downloads. No files were
published and the installed application was unchanged by this validation.


## Build 62 source archive

The local `libretype-build62-source.tar.gz` archive was exported from revision
`68ba53bf31eee82ec4449551f48934255f214503`, matching the source used for build 62.
All 12,845 manifest file hashes were verified, including the new MLX backend
probe and changed model sources. The notice inventory is complete. The app
bundle passed packaging verification and was installed with a backup of build 61.

Archive SHA-256:
`7c699b449b43a3a22c8c744389676a49250e2333b2edb9628631ca4afdd1b809`.

The archive then rebuilt successfully with an empty Cargo cache and Cargo
network access disabled. All-target Rust tests passed (202 library and 243
application tests, with overlap), along with 18 packaging tests, 14 companion
tests and the notice inventory check. A bundle rebuilt from the export passed
strict signature verification and byte checks for 145 dependency/runtime notice
files. The temporary build was removed afterward. The signed executables were
not byte-identical to the installed build; this verifies offline buildability,
not bit-for-bit reproducibility.

Native build 62 verification remains pending. Nothing was published, and the
contributor, certificate signing and distribution review gates remain open.


## References

- https://www.gnu.org/licenses/gpl-3.0.html (especially sections 1, 4–6 and 10)
- https://www.gnu.org/licenses/gpl-howto.en.html
- https://www.gnu.org/licenses/gpl-faq.en.html

This is a technical release procedure, not a determination of authorship,
contractual obligations or legal clearance.
