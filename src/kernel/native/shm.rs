use alloc::vec;
use alloc::vec::Vec;

use crate::kernel::sync::SpinLock;

use super::abi::types::{Error, Result, Signals};

#[path = "shm_rect_contrat.rs"]
pub mod rect;

pub const MAX_SHARED_REGION: usize = 64 * 1024 * 1024;

/// Shared kernel region.
///
/// The object is genuinely shared between processes when its handle is passed
/// through a channel.  V1 exposes safe read/write syscalls; the MAP right is
/// already carried by the handle so a later zero-copy mmap backend can replace
/// this storage without changing the ABI.
pub struct SharedRegion {
    bytes: SpinLock<Vec<u8>>,
}

impl SharedRegion {
    pub fn new(size: usize) -> Result<Self> {
        if size == 0 || size > MAX_SHARED_REGION { return Err(Error::TooLarge); }
        Ok(Self { bytes: SpinLock::new(vec![0u8; size]) })
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
