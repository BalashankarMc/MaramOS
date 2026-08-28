use crate::{fs::FSError, memory::PhysPage};
use super::{Disk, FSResult, LemonData};

pub fn read_data<T: LemonData>(disk: Disk, block: u64) -> FSResult<T> {
    let page = disk.read_sectors(block, 1).map_err(|_| FSError::IOError)?;
    page.read_data(0).ok_or(FSError::IOError)
}

pub fn write_data<T: LemonData>(disk: Disk, block: u64, val: T) -> FSResult<()> {
    let page = PhysPage::new(1).ok_or(FSError::IOError)?;
    page.write_data(0, val).ok_or(FSError::IOError)?;
    disk.write_sectors(block, 1, &page).map_err(|_| FSError::IOError)
}