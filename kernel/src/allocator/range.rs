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

    pub fn allocate(&mut self, size: usize) -> Option<usize> {
        for (i, range) in self.free.iter_mut().enumerate() {
            if range.length == size {
                return Some(self.free.remove(i).start);
            } else if range.length > size {
                let res = range.start;
                range.start += size;
                range.length -= size;

                return Some(res)
            }
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