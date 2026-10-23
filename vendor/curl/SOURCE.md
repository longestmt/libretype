# Rust curl bindings

The application uses `curl` 0.4.51 and `curl-sys` 0.4.91+curl-8.22.0,
under the identical MIT license retained in LICENSE. Sources:
https://crates.io/crates/curl/0.4.51 and
https://crates.io/crates/curl-sys/0.4.91+curl-8.22.0.

The macOS build links the system libcurl dynamically. No TLS or remote model
requests are introduced; inference transport accepts numeric loopback HTTP only.
The exact dependency versions and checksums are recorded in Cargo.lock.
