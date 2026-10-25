// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! The small set of 64-bit macOS C ABI types used by the native integration.
//! Foundation's NSGeometry.h aliases NSPoint, NSSize and NSRect to CoreGraphics
//! types on 64-bit macOS; CGFloat is double. No ownership is represented here.
#![allow(non_camel_case_types, non_upper_case_globals)]

#[cfg(not(target_pointer_width = "64"))]
compile_error!("LibreType's macOS integration requires a 64-bit target");

pub type id = *mut objc::runtime::Object;
pub const nil: id = std::ptr::null_mut();

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NSPoint {
    pub x: f64,
    pub y: f64,
}
impl NSPoint {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NSSize {
    pub width: f64,
    pub height: f64,
}
impl NSSize {
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NSRect {
    pub origin: NSPoint,
    pub size: NSSize,
}
impl NSRect {
    pub const fn new(origin: NSPoint, size: NSSize) -> Self {
        Self { origin, size }
    }
}

// SAFETY: repr(C), field order and f64 members match the 64-bit macOS SDK.
unsafe impl objc::Encode for NSPoint {
    fn encode() -> objc::Encoding {
        unsafe { objc::Encoding::from_str("{CGPoint=dd}") }
    }
}
// SAFETY: repr(C) and two doubles match CGSize in the 64-bit macOS SDK.
unsafe impl objc::Encode for NSSize {
    fn encode() -> objc::Encoding {
        unsafe { objc::Encoding::from_str("{CGSize=dd}") }
    }
}
// SAFETY: CGRect consists of CGPoint followed by CGSize, both encoded above.
unsafe impl objc::Encode for NSRect {
    fn encode() -> objc::Encoding {
        unsafe { objc::Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
    }
}

#[cfg(test)]
mod tests {
    #![allow(unexpected_cfgs)]
    use super::*;
    use objc::{class, msg_send, sel, sel_impl, Encode};
    use std::{
        ffi::{c_char, CStr, CString},
        mem::{align_of, size_of},
    };

    #[link(name = "Foundation", kind = "framework")]
    extern "C" {
        fn NSGetSizeAndAlignment(
            encoding: *const c_char,
            size: *mut usize,
            align: *mut usize,
        ) -> *const c_char;
    }

    fn verify_layout<T: Encode>() {
        let encoding = CString::new(T::encode().as_str()).unwrap();
        let (mut size, mut align) = (0, 0);
        unsafe {
            NSGetSizeAndAlignment(encoding.as_ptr(), &mut size, &mut align);
        }
        assert_eq!(size, size_of::<T>());
        assert_eq!(align, align_of::<T>());
    }

    #[test]
    fn foundation_geometry_layout_and_message_roundtrips() {
        verify_layout::<NSPoint>();
        verify_layout::<NSSize>();
        verify_layout::<NSRect>();
        unsafe {
            let pool: id = msg_send![class!(NSAutoreleasePool), new];
            let point = NSPoint::new(-27.25, 91.5);
            let size = NSSize::new(123.125, 47.75);
            let rect = NSRect::new(point, size);
            let p: id = msg_send![class!(NSValue), valueWithPoint:point];
            let s: id = msg_send![class!(NSValue), valueWithSize:size];
            let r: id = msg_send![class!(NSValue), valueWithRect:rect];
            let actual_point: NSPoint = msg_send![p, pointValue];
            let actual_size: NSSize = msg_send![s, sizeValue];
            let actual_rect: NSRect = msg_send![r, rectValue];
            assert_eq!(actual_point, point);
            assert_eq!(actual_size, size);
            assert_eq!(actual_rect, rect);
            let encoding: *const c_char = msg_send![r, objCType];
            assert_eq!(
                CStr::from_ptr(encoding).to_str().unwrap(),
                NSRect::encode().as_str()
            );
            let _: () = msg_send![pool, drain];
        }
    }
}
