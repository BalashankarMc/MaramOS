use alloc::vec::Vec;

#[derive(Debug, Clone, Copy)]
struct Range {
    start: usize,
    length: usize
}

#[derive(Debug)]
pub struct RangeAllocator {
    free: Vec<Range>
}

impl RangeAllocator {
    pub const fn new() -> Self { Self { free: Vec::new() } }

    pub fn allocate(&mut self, size: usize, align: usize) -> Option<usize> {
        for i in 0..self.free.len() {
            let range = self.free[i];
            let aligned_start = range.start.next_multiple_of(align);
            if aligned_start + size > range.start + range.length { continue }

            self.free.remove(i);

            if aligned_start > range.start { self.free.push(Range { start: range.start, length: aligned_start - range.start }) }
            let tail = range.start + range.length - (aligned_start + size);
            if tail != 0 { self.free.push(Range { start: aligned_start + size, length: tail }) }
            
            self.coalesce();

            return Some(aligned_start);
        }
        None
    }

    pub fn add_range(&mut self, start: usize, end: usize) {
        self.free.push(Range { start, length: end - start });
        self.coalesce();
    }

    pub fn free(&mut self, base: usize, size: usize) {
        self.free.push(Range { start: base, length: size });
        self.coalesce();
    }

    pub fn reserve(&mut self, start: usize, size: usize) -> bool {
        let Some(end) = start.checked_add(size) else { return false };
        if start == end { return true }

        self.coalesce();

        let mut cursor = start;
        for range in &self.free {
            let range_end = range.start + range.length;
            if range_end <= cursor { continue }
            if range.start > cursor { return false }

            cursor = range_end;
            if cursor >= end { break }
        }
        
        if cursor < end { return false }
        
        let mut kept = Vec::with_capacity(self.free.len() + 1);
        for range in self.free.drain(..) {
            let range_end = range.start + range.length;
            
            if range_end <= start || range.start >= end { kept.push(range) }
            else {
                if range.start < start { kept.push(Range { start: range.start, length: start - range.start }) }
                if range_end > end { kept.push(Range { start: end, length: range_end - end }) }
            }
        }

        self.free = kept;
        self.coalesce();

        true
    }

    fn coalesce(&mut self) {
        self.free.sort_by_key(|r| r.start);

        let mut i = 0;
        while i + 1 < self.free.len() {
            let curr_end = self.free[i].start + self.free[i].length;
            let next = &self.free[i + 1];

            if curr_end == next.start {
                self.free[i].length += next.length;
                self.free.remove(i + 1);
            } else { i += 1 }
        }
    }
}