use axhal::mem::PAGE_SIZE_4K;
use axhal::paging::MappingFlags;
use axmm::AddrSpace;
use std::{fs, io};

use crate::VM_ENTRY;

pub fn load_vm_image(fname: &str, uspace: &mut AddrSpace) -> io::Result<()> {
    let data = fs::read(fname)?;
    assert!(data.len() <= PAGE_SIZE_4K);
    let entry = VM_ENTRY.into();
    uspace.map_alloc(
        entry,
        PAGE_SIZE_4K,
        MappingFlags::READ | MappingFlags::EXECUTE | MappingFlags::USER,
        true,
    )?;
    uspace.write(entry, &data)?;
    Ok(())
}
