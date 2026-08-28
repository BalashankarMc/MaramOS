use alloc::{vec::Vec, vec};

use super::{Disk, FSResult, LemonData, disk::{LinkBlock, LinkPointer}, io::read_data};

fn resolve_link(disk: Disk, link: LinkPointer) -> FSResult<Vec<LinkPointer>> {
    if !link.is_valid() { return Ok(Vec::new()) }
    if link.is_direct() { return Ok(vec![link]) }

    let mut res = Vec::with_capacity(32);
    let block: LinkBlock = link.read(disk)?;

    res.extend(block.0[..31].iter().filter(|link| link.is_valid() && link.is_direct()));
    let last = &block.0[31];
    if last.is_valid() { res.push(*last) }

    Ok(res)
}

/// Takes a `LinkPointer` `start` and traverses it's `LinkBlock` chain, returning the direct pointers
pub fn resolve_links(disk: Disk, start: LinkPointer) -> FSResult<Vec<LinkPointer>> {
    let mut stack = vec![start];

    while let Some(link) = stack.pop() {
        if !link.is_valid() { break }
        if link.is_direct() {
            stack.push(link);
            break
        }

        stack.extend(resolve_link(disk, link)?);
    }

    Ok(stack)
}

/// Takes a `LinkPointer` `start` and traces it's full path,
/// returning both the direct and indirect links, in that order.
pub fn trace(disk: Disk, start: LinkPointer) -> FSResult<(Vec<LinkPointer>, Vec<LinkPointer>)> {
    let mut direct_pointers = Vec::new();
    let mut indirect_pointers = Vec::new();

    let mut ptr = start;
    while ptr.is_valid() {
        if ptr.is_direct() { direct_pointers.push(ptr); break }
        let block: LinkBlock = ptr.read(disk)?;

        direct_pointers.extend(block.0[..31].iter().filter(|link| link.is_valid() && link.is_direct()));
        let last = block.0[31];
        indirect_pointers.push(ptr);
        ptr = last;
    }

    Ok((direct_pointers, indirect_pointers))
}

impl LinkPointer {
    pub const fn is_valid(&self) -> bool { self.start != 0 }
    pub const fn is_direct(&self) -> bool { self.size != 0 }

    pub const fn new(start: u64, size: u64) -> Self { Self { start, size } }
    pub fn read<T: LemonData>(&self, disk: Disk) -> FSResult<T> { read_data(disk, self.start) }
}