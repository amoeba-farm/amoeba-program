//! An upward bump allocator over the program heap.
//!
//! The SDK's allocator hands out memory from the top of a fixed-size heap, so
//! a program that wants more than the default 32 KiB must have every caller
//! request the larger frame. This one grows from the bottom: instructions that
//! allocate little work with the default heap, and only `Submit` and `Quote`
//! (which rebuild the whole book) need a `RequestHeapFrame` of up to
//! [`HEAP_MAX`]. Allocating past the frame actually granted faults the
//! transaction, which then rolls back atomically.
#![allow(unsafe_code)]

/// The largest heap frame the runtime grants.
pub const HEAP_MAX: usize = 256 * 1024;

#[cfg(all(target_os = "solana", feature = "custom-heap"))]
mod imp {
    use super::HEAP_MAX;
    use solana_program::entrypoint::HEAP_START_ADDRESS;
    use std::alloc::{GlobalAlloc, Layout};

    pub struct UpwardBump;

    unsafe impl GlobalAlloc for UpwardBump {
        #[inline]
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let start = HEAP_START_ADDRESS as usize;
            let cursor = start as *mut usize;
            // The heap is zeroed per invocation; the first word is the cursor.
            let mut pos = *cursor;
            if pos == 0 {
                pos = start + core::mem::size_of::<usize>();
            }
            let at = (pos + layout.align() - 1) & !(layout.align() - 1);
            let end = match at.checked_add(layout.size()) {
                Some(e) if e <= start + HEAP_MAX => e,
                _ => return core::ptr::null_mut(),
            };
            *cursor = end;
            at as *mut u8
        }
        #[inline]
        unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
    }

    #[global_allocator]
    static ALLOCATOR: UpwardBump = UpwardBump;
}
