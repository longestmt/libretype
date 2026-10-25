// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Chrome Native Messaging shim; only forwards bounded geometry messages locally.
#[cfg(unix)]
fn main() {
    use libretypist::docs_bridge::{self as bridge, Reply, Request};
    let expected = format!("chrome-extension://{}/", bridge::EXTENSION_ID.trim());
    if std::env::args().nth(1).as_deref() != Some(expected.as_str()) {
        std::process::exit(1);
    }
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    while let Ok(Some(bytes)) = bridge::read_frame(&mut input) {
        let reply = serde_json::from_slice::<Request>(&bytes)
            .ok()
            .and_then(|request| {
                bridge::transport::socket_path()
                    .ok()
                    .and_then(|path| bridge::transport::exchange(&path, &request).ok())
            })
            .unwrap_or(Reply {
                enabled: false,
                accepted: false,
            });
        if bridge::write_frame(&mut output, &reply).is_err() {
            break;
        }
    }
}
#[cfg(not(unix))]
fn main() {
    std::process::exit(1);
}
