//! An upward bump allocator keeps small instructions inside the default 32 KiB heap.
//! Strip clients request the larger frame explicitly. The VM enforces the actual frame;
//! this allocator additionally refuses allocations beyond Solana's 256 KiB maximum.

#[cfg(any(test, all(not(feature = "no-entrypoint"), target_os = "solana")))]
fn allocation_range(
    start: usize,
    length: usize,
    cursor: usize,
    size: usize,
    align: usize,
) -> Option<(usize, usize)> {
    if !align.is_power_of_two() {
        return None;
    }
    let first = start.checked_add(core::mem::size_of::<usize>())?;
    let end = start.checked_add(length)?;
    let current = if cursor == 0 { first } else { cursor };
    if current < first || current > end {
        return None;
    }
    let aligned = current.checked_add(align - 1)? & !(align - 1);
    let next = aligned.checked_add(size.max(1))?;
    if next > end {
        return None;
    }
    Some((aligned, next))
}

#[cfg(all(not(feature = "no-entrypoint"), target_os = "solana"))]
struct BoundedHeap;

#[cfg(all(not(feature = "no-entrypoint"), target_os = "solana"))]
unsafe impl core::alloc::GlobalAlloc for BoundedHeap {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let start = solana_program::entrypoint::HEAP_START_ADDRESS as usize;
        let cursor = start as *mut usize;
        // The loader supplies a zeroed heap for this invocation. The first word is
        // reserved for this allocator, and allocations never overlap that word.
        let previous = unsafe { cursor.read() };
        let Some((pointer, next)) =
            allocation_range(start, 262_144, previous, layout.size(), layout.align())
        else {
            return core::ptr::null_mut();
        };
        unsafe { cursor.write(next) };
        pointer as *mut u8
    }

    unsafe fn dealloc(&self, _: *mut u8, _: core::alloc::Layout) {}
}

#[cfg(all(not(feature = "no-entrypoint"), target_os = "solana"))]
#[global_allocator]
static ALLOCATOR: BoundedHeap = BoundedHeap;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocations_are_aligned_disjoint_and_start_in_default_heap() {
        let start = 0x300000000usize;
        let mut cursor = 0;
        let mut previous_end = start + core::mem::size_of::<usize>();
        for align in [1, 2, 4, 8, 16, 32, 64, 128, 4096] {
            for size in [0, 1, 3, 68, 128] {
                let (pointer, next) = allocation_range(start, 32_768, cursor, size, align).unwrap();
                assert_eq!(pointer % align, 0);
                assert!(pointer >= previous_end);
                assert_eq!(next - pointer, size.max(1));
                assert!(next <= start + 32_768);
                cursor = next;
                previous_end = next;
            }
        }
    }

    #[test]
    fn exhaustion_invalid_cursor_and_overflow_fail_closed() {
        let start = 1024;
        let length = 262_144;
        let first = start + core::mem::size_of::<usize>();
        assert_eq!(
            allocation_range(start, length, 0, length - core::mem::size_of::<usize>(), 1),
            Some((first, start + length))
        );
        for cursor in [start, start + length + 1, usize::MAX] {
            assert_eq!(allocation_range(start, length, cursor, 1, 1), None);
        }
        assert_eq!(allocation_range(start, length, start + length, 1, 1), None);
        assert_eq!(allocation_range(start, length, 0, usize::MAX, 1), None);
        assert_eq!(allocation_range(usize::MAX - 4, length, 0, 1, 1), None);
        assert_eq!(allocation_range(start, length, 0, 1, 0), None);
        assert_eq!(allocation_range(start, length, 0, 1, 3), None);
        assert_eq!(
            allocation_range(start, length, 0, 1, usize::MAX / 2 + 1),
            None
        );
    }
}
