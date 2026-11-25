use alloc::vec::Vec;

use crate::kernel::sync::SpinLock;

use super::abi::types::{Error, Result, Signals};

#[path = "shm_rect_contrat.rs"]
pub mod rect;

pub const MAX_SHARED_REGION: usize = 64 * 1024 * 1024;

/// Shared kernel region.
///
/// The object is genuinely shared between processes when its handle is passed
/// through a channel. Since ABI 1.7 (BOUCHAUD_SHM_PROJECTION_V1) it can also be
/// MAPPED into a process (`bo_shm_map`): the storage is a page-aligned,
/// page-granular block of the kernel heap arena, which is physically
/// contiguous, so its frames are projected as-is -- no copy, and the kernel
/// keeps its contiguous view (`avec_octets*`, used by the compositor).
pub struct SharedRegion {
    bytes: SpinLock<Tampon>,
}

/// Page-aligned zeroed storage, rounded up to whole pages: nothing else of the
/// kernel heap can share a frame with a mapped region.
pub struct Tampon {
    ptr: core::ptr::NonNull<u8>,
    len: usize,
    alloue: usize,
}

// The buffer is owned exclusively by its region; access goes through the lock.
unsafe impl Send for Tampon {}
unsafe impl Sync for Tampon {}

const PAGE: usize = 4096;

impl Tampon {
    fn nouveau(len: usize) -> Result<Self> {
        let alloue = len.checked_add(PAGE - 1).ok_or(Error::TooLarge)? & !(PAGE - 1);
        let layout = core::alloc::Layout::from_size_align(alloue, PAGE).map_err(|_| Error::TooLarge)?;
        let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) };
        let ptr = core::ptr::NonNull::new(ptr).ok_or(Error::NoSpace)?;
        Ok(Self { ptr, len, alloue })
    }
}

impl core::ops::Deref for Tampon {
    type Target = [u8];
    fn deref(&self) -> &[u8] { unsafe { core::slice::from_raw_parts(self.ptr.as_ptr(), self.len) } }
}

impl core::ops::DerefMut for Tampon {
    fn deref_mut(&mut self) -> &mut [u8] { unsafe { core::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) } }
}

impl Drop for Tampon {
    fn drop(&mut self) {
        let layout = core::alloc::Layout::from_size_align(self.alloue, PAGE).expect("shm: layout");
        unsafe { alloc::alloc::dealloc(self.ptr.as_ptr(), layout) }
    }
}

impl SharedRegion {
    pub fn new(size: usize) -> Result<Self> {
        if size == 0 || size > MAX_SHARED_REGION { return Err(Error::TooLarge); }
        Ok(Self { bytes: SpinLock::new(Tampon::nouveau(size)?) })
    }

    /// BOUCHAUD_SHM_PROJECTION_V1 : (adresse physique, octets projetables --
    /// la taille arrondie a la page). `None` si le stockage n'est pas dans
    /// l'arene physique du tas (il n'est alors jamais projete).
    pub fn projection(&self) -> Option<(u64, u64)> {
        let tampon = self.bytes.lock();
        let phys = crate::kernel::heap::phys_dans_arene(tampon.ptr.as_ptr(), tampon.alloue)?;
        (phys % PAGE as u64 == 0).then_some((phys, tampon.alloue as u64))
    }

    pub fn len(&self) -> usize { self.bytes.lock().len() }

    pub fn read(&self, offset: usize, len: usize) -> Result<Vec<u8>> {
        let bytes = self.bytes.lock();
        let end = offset.checked_add(len).ok_or(Error::InvalidArgument)?;
        if end > bytes.len() { return Err(Error::InvalidArgument); }
        Ok(bytes[offset..end].to_vec())
    }

    pub fn write(&self, offset: usize, data: &[u8]) -> Result<()> {
        let mut bytes = self.bytes.lock();
        let end = offset.checked_add(data.len()).ok_or(Error::InvalidArgument)?;
        if end > bytes.len() { return Err(Error::InvalidArgument); }
        bytes[offset..end].copy_from_slice(data);
        Ok(())
    }

    /// Acces en place, sous le verrou de la region (BOUCHAUD_BUREAU_RING3_2A) :
    /// le noyau y recopie la surface d'une application sans tampon
    /// intermediaire. L'appelant borne lui-meme ses ecritures (`fenetre_contrat`).
    pub fn avec_octets_mut<R>(&self, f: impl FnOnce(&mut [u8]) -> R) -> R {
        let mut bytes = self.bytes.lock();
        f(&mut bytes)
    }

    /// Lecture en place, sous le verrou de la region : le noyau en pose les
    /// lignes sans tampon intermediaire (`affichage::present`).
    pub fn avec_octets<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        let bytes = self.bytes.lock();
        f(&bytes)
    }

    /// BOUCHAUD_SHM_RECT_V1 : `bo_shm_fill_rect`. Valide sous le verrou (la
    /// taille de la region y est lue), puis remplit sur place : ni copie
    /// depuis l'appelant, ni allocation. Rend le nombre de pixels ecrits.
    pub fn remplit_rect(&self, decalage: u64, couleur: u64, pixels: u64, lignes: u64, pas: u64) -> Result<u64> {
        let mut bytes = self.bytes.lock();
        let r = rect::valide(decalage, couleur, pixels, lignes, pas, bytes.len())
            .map_err(|_| Error::InvalidArgument)?;
        rect::applique(&r, &mut bytes);
        Ok(pixels * lignes)
    }

    pub fn signals(&self) -> Signals { Signals::READABLE | Signals::WRITABLE }
}
