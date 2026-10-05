//! An upward bump allocator keeps small instructions inside the default 32 KiB heap.
//! Strip clients request the larger frame explicitly. The VM enforces the actual frame;
//! this allocator additionally refuses allocations beyond Solana's 256 KiB maximum.

#[cfg(all(not(feature = "no-entrypoint"), target_os = "solana"))]
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
