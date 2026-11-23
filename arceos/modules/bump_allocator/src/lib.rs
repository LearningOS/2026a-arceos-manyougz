#![no_std]

use allocator::{AllocError, AllocResult, BaseAllocator, ByteAllocator, PageAllocator};
use core::{alloc::Layout, ptr::NonNull};

/// Early memory allocator
/// Use it before formal bytes-allocator and pages-allocator can work!
/// This is a double-end memory range:
/// - Alloc bytes forward
/// - Alloc pages backward
///
/// [ bytes-used | avail-area | pages-used ]
/// |            | -->    <-- |            |
/// start       b_pos        p_pos       end
///
/// For bytes area, 'count' records number of allocations.
/// When it goes down to ZERO, free bytes-used area.
/// For pages area, it will never be freed!
///
pub struct EarlyAllocator<const SIZE: usize> {
    start: usize,
    end: usize,
    b_pos: usize,
    p_pos: usize,
    count: usize,
}

impl<const SIZE: usize> EarlyAllocator<SIZE> {
    pub const fn new() -> Self {
        Self {
            start: 0,
            end: 0,
            b_pos: 0,
            p_pos: 0,
            count: 0,
        }
    }
}

impl<const SIZE: usize> BaseAllocator for EarlyAllocator<SIZE> {
    fn init(&mut self, start: usize, size: usize) {
        self.start = start;
        self.end = start + size;
        self.b_pos = start;
        self.p_pos = self.end;
        self.count = 0;
    }

    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult {
        if start != self.end || self.p_pos != self.end {
            return Err(AllocError::InvalidParam);
        }
        self.end = start.checked_add(size).ok_or(AllocError::InvalidParam)?;
        self.p_pos = self.end;
        Ok(())
    }
}

impl<const SIZE: usize> ByteAllocator for EarlyAllocator<SIZE> {
    fn alloc(&mut self, layout: Layout) -> AllocResult<NonNull<u8>> {
        let pos = self.b_pos.max(1);
        let padding = pos.wrapping_neg() & (layout.align() - 1);
        let start = pos.checked_add(padding).ok_or(AllocError::NoMemory)?;
        let end = start
            .checked_add(layout.size())
            .ok_or(AllocError::NoMemory)?;
        if end > self.p_pos {
            return Err(AllocError::NoMemory);
        }
        self.b_pos = end;
        self.count += 1;
        Ok(NonNull::new(start as *mut u8).unwrap())
    }

    fn dealloc(&mut self, _pos: NonNull<u8>, _layout: Layout) {
        self.count -= 1;
        if self.count == 0 {
            self.b_pos = self.start;
        }
    }

    fn total_bytes(&self) -> usize {
        self.end - self.start
    }

    fn used_bytes(&self) -> usize {
        self.b_pos - self.start
    }

    fn available_bytes(&self) -> usize {
        self.p_pos - self.b_pos
    }
}

impl<const SIZE: usize> PageAllocator for EarlyAllocator<SIZE> {
    const PAGE_SIZE: usize = SIZE;

    fn alloc_pages(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        if num_pages == 0 || !align_pow2.is_power_of_two() {
            return Err(AllocError::InvalidParam);
        }
        let align = align_pow2.max(SIZE);
        let size = num_pages.checked_mul(SIZE).ok_or(AllocError::NoMemory)?;
        let start = self.p_pos.checked_sub(size).ok_or(AllocError::NoMemory)? & !(align - 1);
        if start < self.b_pos {
            return Err(AllocError::NoMemory);
        }
        self.p_pos = start;
        Ok(start)
    }

    fn dealloc_pages(&mut self, _pos: usize, _num_pages: usize) {}

    fn total_pages(&self) -> usize {
        ((self.end & !(SIZE - 1)).saturating_sub(self.start.next_multiple_of(SIZE))) / SIZE
    }

    fn used_pages(&self) -> usize {
        (self.end & !(SIZE - 1)).saturating_sub(self.p_pos) / SIZE
    }

    fn available_pages(&self) -> usize {
        (self.p_pos & !(SIZE - 1)).saturating_sub(self.b_pos.next_multiple_of(SIZE)) / SIZE
    }
}
