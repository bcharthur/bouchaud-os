//! Small SMP-safe locking primitives for the kernel.
//!
//! NG1 introduces only non-sleeping locks. Sleeping mutexes and wait queues are
//! added in the next migration step once task blocking is converted to an
//! explicit wakeup model.

use core::arch::asm;
use core::cell::UnsafeCell;
use core::hint::spin_loop;
use core::ops::{Deref, DerefMut};
use core::panic::Location;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::arch::x86_64::smp;

const NO_OWNER: usize = usize::MAX;

// ── Qui tourne, sur quel verrou, depuis ou ────────────────────────────────────
//
// Un CPU bloque dans `lock()` ne laisse aucune trace ailleurs : il n'acquiert
// pas le BKL, ne prend pas de faute, ne change pas de tache. Vu de la sonde, un
// noyau fige sur un verrou tournant et un noyau occupe a calculer se
// ressemblent trait pour trait — sauf que le premier ne finira jamais.
//
// `lock()` porte deja `#[track_caller]` : le site d'appel est disponible sans
// rien couter. On le publie des que l'attente depasse le seuil, et on ne le
// retire qu'a l'acquisition du meme verrou, de sorte qu'une interruption qui
// prend brievement un autre verrou n'efface pas la trace du contexte
// interrompu.
//
// Le chemin non contendu ne paie rien : le compteur et la publication vivent
// dans la boucle d'attente, qui ne s'execute que si le verrou est deja pris.

pub const ATTENTE_ABSENTE: u32 = 0;
/// Attente anormalement longue : le proprietaire existe et ne rend pas la main.
pub const ATTENTE_LONGUE: u32 = 1;
/// Reprise du meme verrou par le CPU qui le detient deja : blocage certain.
///
/// `lock()` n'a jamais ete reentrant ; la verification etait un `debug_assert!`,
/// donc absente du noyau qui tourne reellement. Une reprise recursive y boucle
/// en silence pour toujours.
pub const ATTENTE_REENTRANTE: u32 = 2;

const TOURS_AVANT_SIGNALEMENT: u32 = 1 << 16;

struct Attente {
    etat: AtomicU32,
    verrou: AtomicUsize,
    proprietaire: AtomicUsize,
    fichier: AtomicUsize,
    fichier_len: AtomicUsize,
    ligne: AtomicU32,
    depuis: AtomicU64,
}

impl Attente {
    const fn new() -> Self {
        Self {
            etat: AtomicU32::new(ATTENTE_ABSENTE),
            verrou: AtomicUsize::new(0),
            proprietaire: AtomicUsize::new(NO_OWNER),
            fichier: AtomicUsize::new(0),
            fichier_len: AtomicUsize::new(0),
            ligne: AtomicU32::new(0),
            depuis: AtomicU64::new(0),
        }
    }
}

static ATTENTES: [Attente; smp::MAX_CPUS] =
    [const { Attente::new() }; smp::MAX_CPUS];

/// Etat d'attente publie par un CPU, tel que la sonde le lit.
pub struct AttenteVerrou {
    pub etat: u32,
    pub verrou: usize,
    pub proprietaire: usize,
    pub fichier: &'static str,
    pub ligne: u32,
    pub depuis: u64,
}

#[inline(never)]
fn signale(etat: u32, verrou: usize, proprietaire: usize, ou: &'static Location<'static>) {
    let cpu = smp::cpu_index();
    if cpu >= smp::MAX_CPUS {
        return;
    }
    let attente = &ATTENTES[cpu];
    attente.verrou.store(verrou, Ordering::Relaxed);
    attente.proprietaire.store(proprietaire, Ordering::Relaxed);
    attente.fichier.store(ou.file().as_ptr() as usize, Ordering::Relaxed);
    attente.fichier_len.store(ou.file().len(), Ordering::Relaxed);
    attente.ligne.store(ou.line(), Ordering::Relaxed);
    attente.depuis.store(crate::kernel::timer::ticks(), Ordering::Relaxed);
    // Publie en dernier : la sonde ne lit les champs qu'apres avoir vu l'etat.
    attente.etat.store(etat, Ordering::Release);
}

#[inline]
fn acquitte(verrou: usize) {
    let cpu = smp::cpu_index();
    if cpu >= smp::MAX_CPUS {
        return;
    }
    let attente = &ATTENTES[cpu];
    // Seul le verrou signale peut lever son propre signalement : sinon une
    // interruption qui acquiert autre chose effacerait la trace du contexte
    // qu'elle interrompt, exactement le defaut qui rendait `site=` muet.
    if attente.verrou.load(Ordering::Relaxed) == verrou {
        attente.etat.store(ATTENTE_ABSENTE, Ordering::Release);
    }
}

/// Attente publiee par `cpu`, si ce CPU tourne sur un verrou depuis longtemps.
pub fn attente_verrou(cpu: usize) -> Option<AttenteVerrou> {
    if cpu >= smp::MAX_CPUS {
        return None;
    }
    let attente = &ATTENTES[cpu];
    let etat = attente.etat.load(Ordering::Acquire);
    if etat == ATTENTE_ABSENTE {
        return None;
    }
    let ptr = attente.fichier.load(Ordering::Relaxed);
    let len = attente.fichier_len.load(Ordering::Relaxed);
    // Le pointeur vient d'un `&'static str` de `Location`, donc valide pour
    // toute la duree du noyau. On refuse quand meme un champ jamais ecrit.
    let fichier = if ptr == 0 || len == 0 || len > 256 {
        "?"
    } else {
        unsafe {
            core::str::from_utf8_unchecked(core::slice::from_raw_parts(
                ptr as *const u8,
                len,
            ))
        }
    };
    Some(AttenteVerrou {
        etat,
        verrou: attente.verrou.load(Ordering::Relaxed),
        proprietaire: attente.proprietaire.load(Ordering::Relaxed),
        fichier,
        ligne: attente.ligne.load(Ordering::Relaxed),
        depuis: attente.depuis.load(Ordering::Relaxed),
    })
}

pub struct SpinLock<T: ?Sized> {
    locked: AtomicBool,
    owner_cpu: AtomicUsize,
    value: UnsafeCell<T>,
}

unsafe impl<T: ?Sized + Send> Send for SpinLock<T> {}
unsafe impl<T: ?Sized + Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            owner_cpu: AtomicUsize::new(NO_OWNER),
            value: UnsafeCell::new(value),
        }
    }

    pub fn into_inner(self) -> T {
        self.value.into_inner()
    }
}

impl<T: ?Sized> SpinLock<T> {
    #[inline]
    fn current_cpu() -> usize {
        smp::cpu_index()
    }

    #[track_caller]
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        let cpu = Self::current_cpu();

        debug_assert!(
            self.owner_cpu.load(Ordering::Relaxed) != cpu,
            "SpinLock recursive acquisition on CPU {}",
            cpu
        );

        let verrou = self as *const Self as *const () as usize;
        let mut tours = 0u32;
        let mut signale_publie = false;

        loop {
            if self
                .locked
                .compare_exchange_weak(
                    false,
                    true,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                self.owner_cpu.store(cpu, Ordering::Relaxed);
                if signale_publie {
                    acquitte(verrou);
                }
                return SpinLockGuard { lock: self };
            }

            while self.locked.load(Ordering::Relaxed) {
                spin_loop();
                if signale_publie {
                    continue;
                }
                tours = tours.wrapping_add(1);
                if tours >= TOURS_AVANT_SIGNALEMENT {
                    let proprietaire = self.owner_cpu.load(Ordering::Relaxed);
                    let etat = if proprietaire == cpu {
                        ATTENTE_REENTRANTE
                    } else {
                        ATTENTE_LONGUE
                    };
                    signale(etat, verrou, proprietaire, Location::caller());
                    signale_publie = true;
                }
            }
        }
    }

    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        let cpu = Self::current_cpu();

        if self.owner_cpu.load(Ordering::Relaxed) == cpu {
            return None;
        }

        if self
            .locked
            .compare_exchange(
                false,
                true,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_err()
        {
            return None;
        }

        self.owner_cpu.store(cpu, Ordering::Relaxed);
        Some(SpinLockGuard { lock: self })
    }

    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    pub fn owner_cpu(&self) -> Option<usize> {
        let owner = self.owner_cpu.load(Ordering::Relaxed);
        if owner == NO_OWNER { None } else { Some(owner) }
    }
}

pub struct SpinLockGuard<'a, T: ?Sized> {
    lock: &'a SpinLock<T>,
}

impl<T: ?Sized> Deref for SpinLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.lock.value.get() }
    }
}

impl<T: ?Sized> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T: ?Sized> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.owner_cpu.store(NO_OWNER, Ordering::Relaxed);
        self.lock.locked.store(false, Ordering::Release);
    }
}

/// Spin lock variant for data shared with interrupt handlers.
///
/// Interrupts are disabled before acquiring the inner spin lock and restored to
/// their previous state only after the lock has been released.
pub struct SpinLockIrq<T: ?Sized> {
    inner: SpinLock<T>,
}

unsafe impl<T: ?Sized + Send> Send for SpinLockIrq<T> {}
unsafe impl<T: ?Sized + Send> Sync for SpinLockIrq<T> {}

impl<T> SpinLockIrq<T> {
    pub const fn new(value: T) -> Self {
        Self {
            inner: SpinLock::new(value),
        }
    }
}

impl<T: ?Sized> SpinLockIrq<T> {
    #[inline]
    fn interrupts_enabled() -> bool {
        let flags: u64;
        unsafe {
            asm!(
                "pushfq",
                "pop {}",
                out(reg) flags,
                options(nomem, preserves_flags)
            );
        }
        flags & (1 << 9) != 0
    }

    #[track_caller]
    pub fn lock(&self) -> SpinLockIrqGuard<'_, T> {
        let restore_interrupts = Self::interrupts_enabled();
        unsafe { asm!("cli", options(nomem, nostack, preserves_flags)); }
        let guard = self.inner.lock();

        SpinLockIrqGuard {
            guard: Some(guard),
            restore_interrupts,
        }
    }

    #[track_caller]
    pub fn try_lock(&self) -> Option<SpinLockIrqGuard<'_, T>> {
        let restore_interrupts = Self::interrupts_enabled();
        unsafe { asm!("cli", options(nomem, nostack, preserves_flags)); }

        match self.inner.try_lock() {
            Some(guard) => Some(SpinLockIrqGuard {
                guard: Some(guard),
                restore_interrupts,
            }),
            None => {
                if restore_interrupts {
                    unsafe { asm!("sti", options(nomem, nostack, preserves_flags)); }
                }
                None
            }
        }
    }

    pub fn is_locked(&self) -> bool {
        self.inner.is_locked()
    }
}

pub struct SpinLockIrqGuard<'a, T: ?Sized> {
    guard: Option<SpinLockGuard<'a, T>>,
    restore_interrupts: bool,
}

impl<T: ?Sized> Deref for SpinLockIrqGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.guard.as_ref().expect("SpinLockIrqGuard released").deref()
    }
}

impl<T: ?Sized> DerefMut for SpinLockIrqGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard
            .as_mut()
            .expect("SpinLockIrqGuard released")
            .deref_mut()
    }
}

impl<T: ?Sized> Drop for SpinLockIrqGuard<'_, T> {
    fn drop(&mut self) {
        // Release the lock before re-enabling interrupts. Otherwise an IRQ on
        // this CPU could immediately recurse into the protected structure.
        drop(self.guard.take());

        if self.restore_interrupts {
            unsafe { asm!("sti", options(nomem, nostack, preserves_flags)); }
        }
    }
}
