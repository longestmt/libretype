// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Compatibility adapter for Objective-C runtime-owned arrays.
//!
//! Allocation ownership and deallocation are provided by malloced. A null empty
//! result is represented without an allocation, rather than a synthetic pointer.
use malloced::Malloced;
use std::ops::Deref;

/// An immutable array returned by an Objective-C runtime copy operation.
pub struct MallocBuffer<T>(Option<Malloced<[T]>>);

impl<T: Copy> MallocBuffer<T> {
    /// Take ownership of a C allocation, or an empty null result.
    ///
    /// # Safety
    /// A non-null pointer must satisfy `Malloced::slice_from_raw_parts`: it must
    /// be aligned, malloc-owned and valid for `len` initialized elements, with
    /// exclusive allocation ownership transferred to this buffer.
    pub unsafe fn new(pointer: *mut T, len: usize) -> Option<Self> {
        if pointer.is_null() {
            return if len == 0 { Some(Self(None)) } else { None };
        }
        Some(Self(Some(Malloced::slice_from_raw_parts(pointer, len))))
    }
}

impl<T> Deref for MallocBuffer<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        self.0.as_deref().unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::MallocBuffer;
    use std::ptr;

    extern "C" {
        fn malloc(size: usize) -> *mut std::ffi::c_void;
    }

    #[test]
    fn absent_runtime_arrays_are_aligned_empty_slices() {
        let empty = unsafe { MallocBuffer::<u128>::new(ptr::null_mut(), 0).unwrap() };
        assert!(empty.is_empty());
        assert_eq!(empty.as_ptr().align_offset(std::mem::align_of::<u128>()), 0);
        assert!(unsafe { MallocBuffer::<u128>::new(ptr::null_mut(), 1) }.is_none());
    }

    #[test]
    fn nonnull_arrays_retain_the_original_allocation_and_length() {
        unsafe {
            let allocation = malloc(2 * std::mem::size_of::<u64>()).cast::<u64>();
            assert!(!allocation.is_null());
            allocation.write(17);
            allocation.add(1).write(29);
            let owner = MallocBuffer::new(allocation, 2).unwrap();
            assert_eq!(&*owner, &[17, 29]);
            assert_eq!(owner.as_ptr(), allocation);
            // Malloced owns this allocation and releases it on drop.
            drop(owner);

            let allocation = malloc(8).cast::<u8>();
            assert!(!allocation.is_null());
            let empty = MallocBuffer::new(allocation, 0).unwrap();
            assert!(empty.is_empty());
            assert_eq!(empty.as_ptr(), allocation);
            drop(empty);
        }
    }
}
