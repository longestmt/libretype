// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Measure synthetic text with local AppKit fonts; never accesses another app.
#![allow(deprecated, unexpected_cfgs)]

#[cfg(target_os = "macos")]
fn main() {
    use libretypist::native_abi::{id, NSSize};
    use objc::{class, msg_send, sel, sel_impl};
    #[link(name = "AppKit", kind = "framework")]
    extern "C" {
        static NSFontAttributeName: id;
    }
    unsafe {
        let pool: id = msg_send![class!(NSAutoreleasePool), new];
        let text: id = msg_send![class!(NSString), alloc];
        let text: id = msg_send![text, initWithBytes:b"your".as_ptr() length:4usize encoding:4usize];
        for points in [12., 14., 16., 18., 20., 24., 32.] {
            for mono in [false, true] {
                let font: id = if mono {
                    msg_send![class!(NSFont), monospacedSystemFontOfSize:points weight:0f64]
                } else {
                    msg_send![class!(NSFont), systemFontOfSize:points]
                };
                let attrs: id = msg_send![class!(NSDictionary), dictionaryWithObject:font forKey:NSFontAttributeName];
                let size: NSSize = msg_send![text, sizeWithAttributes:attrs];
                println!(
                    "{}",
                    serde_json::json!({"points":points,"monospaced":mono,"width":size.width,"height":size.height})
                );
            }
        }
        let _: () = msg_send![text, release];
        let _: () = msg_send![pool, drain];
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This probe requires macOS AppKit.");
}
