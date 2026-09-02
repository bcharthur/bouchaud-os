use alloc::vec;
use alloc::vec::Vec;

use super::types::{Error, Result};

fn resident_range(addr: u64, len: usize, write: bool) -> bool {
    if len == 0 { return true; }
    let last = match addr.checked_add(len as u64 - 1) {
        Some(last) => last,
        None => return false,
    };
    if !crate::kernel::vmm::is_user_addr(addr) || !crate::kernel::vmm::is_user_addr(last) {
        return false;
    }

    let page_size = crate::kernel::vmm::PAGE_SIZE;
    let mut page = addr & !(page_size - 1);
    let last_page = last & !(page_size - 1);

    loop {
        let process = crate::kernel::task::current_process();
        let present = process.mm.lock().space.translate(page).is_some();
        drop(process);

        if !present
            && crate::kernel::task::peuple_a_la_demande(page, false)
                != crate::kernel::task::FaultOutcome::Resolved
        {
            return false;
        }

        if write {
            let process = crate::kernel::task::current_process();
            if !process.mm.lock().space.writable(page) {
                return false;
            }
        }

        if page == last_page { break; }
        page += page_size;
    }
    true
}

pub fn read(addr: u64, len: usize) -> Result<Vec<u8>> {
    if len == 0 { return Ok(Vec::new()); }
    if !resident_range(addr, len, false) { return Err(Error::Fault); }
    let mut out = vec![0u8; len];
    let process = crate::kernel::task::current_process();
    if process.mm.lock().space.read(addr, &mut out) {
        Ok(out)
    } else {
        Err(Error::Fault)
    }
}

pub fn write(addr: u64, bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() { return Ok(()); }
    if !resident_range(addr, bytes.len(), true) { return Err(Error::Fault); }
    let process = crate::kernel::task::current_process();
    if process.mm.lock().space.write(addr, bytes) {
        Ok(())
    } else {
        Err(Error::Fault)
    }
}

pub fn read_handles(addr: u64, count: usize) -> Result<Vec<super::types::HandleId>> {
    if count == 0 { return Ok(Vec::new()); }
    let bytes = read(addr, count.checked_mul(8).ok_or(Error::TooLarge)?)?;
    let mut out = Vec::with_capacity(count);
    for chunk in bytes.chunks_exact(8) {
        let mut raw = [0u8; 8];
        raw.copy_from_slice(chunk);
        out.push(super::types::HandleId::from_raw(u64::from_le_bytes(raw)));
    }
    Ok(out)
}

pub fn write_handles(addr: u64, handles: &[super::types::HandleId]) -> Result<()> {
    let mut bytes = Vec::with_capacity(handles.len() * 8);
    for handle in handles {
        bytes.extend_from_slice(&handle.raw().to_le_bytes());
    }
    write(addr, &bytes)
}
