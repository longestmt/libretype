// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Owned, two-pointer callbacks for the 64-bit Apple Blocks ABI.
//! Uses libSystem for block allocation/refcounting. No third-party block code.
//! ABI reference: https://clang.llvm.org/docs/Block-ABI-Apple.html
use std::{
    ffi::{c_char, c_void},
    ptr::NonNull,
    sync::Arc,
};

type Pointer = *mut c_void;
type Callback = dyn Fn(Pointer, Pointer) + Send + Sync + 'static;
struct State(Box<Callback>);

#[repr(C)]
struct Descriptor {
    reserved: usize,
    size: usize,
    copy: unsafe extern "C" fn(*mut Literal, *const Literal),
    dispose: unsafe extern "C" fn(*const Literal),
    signature: *const c_char,
}
// These descriptors contain only immutable static strings and function pointers.
unsafe impl Sync for Descriptor {}

#[repr(C)]
struct Literal {
    isa: *const c_void,
    flags: i32,
    reserved: i32,
    invoke: unsafe extern "C" fn(*const Literal, Pointer, Pointer),
    descriptor: *const Descriptor,
    state: *const State,
}

#[link(name = "System")]
extern "C" {
    static _NSConcreteStackBlock: c_void;
    fn _Block_copy(block: *const c_void) -> Pointer;
    fn _Block_release(block: *const c_void);
}

unsafe extern "C" fn copy(destination: *mut Literal, source: *const Literal) {
    // libSystem has copied the literal bytes. Each new heap literal owns one
    // Arc reference; further copies of that heap block use its native refcount.
    let state = (*source).state;
    Arc::increment_strong_count(state);
    (*destination).state = state;
}

unsafe extern "C" fn dispose(block: *const Literal) {
    Arc::decrement_strong_count((*block).state);
}

unsafe extern "C" fn invoke(block: *const Literal, first: Pointer, second: Pointer) {
    // The native caller must retain the block throughout invocation. Fn + Sync
    // permits concurrent native callbacks. A Rust panic cannot unwind over C.
    ((*(*block).state).0)(first, second);
}

static OBJECTS: Descriptor = Descriptor {
    reserved: 0,
    size: std::mem::size_of::<Literal>(),
    copy,
    dispose,
    signature: c"v24@?0@8@16".as_ptr(),
};
static IMAGE: Descriptor = Descriptor {
    reserved: 0,
    size: std::mem::size_of::<Literal>(),
    copy,
    dispose,
    signature: c"v24@?0^{CGImage=}8@16".as_ptr(),
};

pub(crate) enum Signature {
    Objects,
    ImageAndError,
}

/// One native heap-block reference. Native APIs must copy it if retaining it.
pub(crate) struct PairBlock(NonNull<Literal>);

impl PairBlock {
    pub(crate) fn new(
        signature: Signature,
        callback: impl Fn(Pointer, Pointer) + Send + Sync + 'static,
    ) -> Self {
        let state = Arc::new(State(Box::new(callback)));
        let literal = Literal {
            isa: std::ptr::addr_of!(_NSConcreteStackBlock),
            flags: (1 << 25) | (1 << 30), // copy/dispose helpers + signature
            reserved: 0,
            invoke,
            descriptor: match signature {
                Signature::Objects => &OBJECTS,
                Signature::ImageAndError => &IMAGE,
            },
            state: Arc::as_ptr(&state),
        };
        // The stack Arc remains alive until _Block_copy has called our helper.
        let copied = unsafe { _Block_copy((&literal as *const Literal).cast()) };
        Self(NonNull::new(copied.cast()).expect("Could not allocate native callback"))
    }

    pub(crate) fn as_ptr(&self) -> Pointer {
        self.0.as_ptr().cast()
    }
}

impl Drop for PairBlock {
    fn drop(&mut self) {
        unsafe { _Block_release(self.as_ptr()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::CString, path::PathBuf, process::Command, sync::Mutex};

    struct Fixture {
        directory: PathBuf,
        library: Pointer,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            unsafe {
                libc::dlclose(self.library);
            }
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
    impl Fixture {
        fn compile() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let directory = std::env::temp_dir()
                .join(format!("libretype-block-{}-{stamp}", std::process::id()));
            std::fs::create_dir(&directory).unwrap();
            let output = directory.join("fixture.dylib");
            let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/block_abi.m");
            let result = Command::new("xcrun")
                .args([
                    "clang",
                    "-dynamiclib",
                    "-fblocks",
                    "-Wall",
                    "-Werror",
                    "-framework",
                    "Foundation",
                    "-framework",
                    "CoreGraphics",
                ])
                .arg(source)
                .arg("-o")
                .arg(&output)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let path = CString::new(output.to_str().unwrap()).unwrap();
            let library = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
            assert!(!library.is_null(), "Could not load native ABI fixture");
            Self { directory, library }
        }
        unsafe fn symbol(&self, name: &std::ffi::CStr) -> Pointer {
            let symbol = libc::dlsym(self.library, name.as_ptr());
            assert!(!symbol.is_null());
            symbol
        }
    }

    #[test]
    fn clang_copies_invokes_on_worker_and_releases_both_callback_signatures() {
        let fixture = Fixture::compile();
        unsafe {
            let retain: unsafe extern "C" fn(Pointer) -> Pointer =
                std::mem::transmute(fixture.symbol(c"lt_copy"));
            let release: unsafe extern "C" fn(Pointer) =
                std::mem::transmute(fixture.symbol(c"lt_release"));
            let objects: unsafe extern "C" fn(Pointer, Pointer, Pointer) -> i32 =
                std::mem::transmute(fixture.symbol(c"lt_objects_on_worker"));
            let image: unsafe extern "C" fn(Pointer, Pointer, Pointer) -> i32 =
                std::mem::transmute(fixture.symbol(c"lt_image_on_worker"));
            for (signature, call) in [
                (Signature::Objects, objects),
                (Signature::ImageAndError, image),
            ] {
                let seen = Arc::new(Mutex::new(Vec::new()));
                let captured = seen.clone();
                let block = PairBlock::new(signature, move |first, second| {
                    captured
                        .lock()
                        .unwrap()
                        .push((first as usize, second as usize));
                });
                let first_copy = retain(block.as_ptr());
                let second_copy = retain(first_copy);
                assert!(!first_copy.is_null() && !second_copy.is_null());
                drop(block);
                assert_eq!(Arc::strong_count(&seen), 2);
                assert_eq!(
                    call(first_copy, std::ptr::null_mut(), std::ptr::null_mut()),
                    0
                );
                release(first_copy);
                assert_eq!(Arc::strong_count(&seen), 2);
                // Pointer values are passed through unchanged; the fixture and
                // callback do not dereference them or treat them as real objects.
                let mut sentinel = 17u64;
                let pointer = (&mut sentinel as *mut u64).cast();
                assert_eq!(call(second_copy, pointer, std::ptr::null_mut()), 0);
                assert_eq!(*seen.lock().unwrap(), vec![(0, 0), (pointer as usize, 0)]);
                release(second_copy);
                assert_eq!(Arc::strong_count(&seen), 1);
            }
        }
    }

    #[test]
    fn unused_callback_releases_capture_without_invocation() {
        let captured = Arc::new(());
        let inside = captured.clone();
        let block = PairBlock::new(Signature::Objects, move |_, _| {
            let _ = &inside;
        });
        assert_eq!(Arc::strong_count(&captured), 2);
        drop(block);
        assert_eq!(Arc::strong_count(&captured), 1);
    }
}
