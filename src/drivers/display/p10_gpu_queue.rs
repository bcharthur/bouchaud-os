//! P10_UNIVERSAL_PRESENT_V1
//!
//! Backend universel de PRESENTATION : captures immuables de la scene Atelier,
//! producteur (compositeur) separe du consommateur (file GPU).
//! TOUS les clients (Ladybird, DOOM, terminal...) arrivent ici via fb::fin_trame.
//!
//! Attention: ceci decouple le blocage virtio-gpu du compositeur. Le
//! rasteriseur CPU Ladybird/Skia n'est PAS deplace sur le GPU.
//!
//! Discipline:
//! - 2 tampons CPU prealloues; jamais de pointeur vers un framebuffer vivant;
//! - une seule trame en attente : remplacement "latest frame wins";
//! - le consommateur prend pleine possession du Vec avant l'attente GPU;
//! - pas de verrou de boite aux lettres tenu pendant DMA ou attente GPU;
//! - le tampon du GPU n'est ecrit que par le consommateur, sous BASCULE;
//! - lorsque l'ecran est quitte, aucune publication n'est encore admise.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use crate::kernel::sync::SpinLock;

// P10 MULTI-WINDOW: les surfaces viennent du compositeur, jamais du processus direct.
#[cfg(feature = "p10-virgl-experimental")]
pub(crate) const MAX_COUCHES: usize = 4;
#[cfg(feature = "p10-virgl-experimental")]
pub(crate) struct Couche {
    pub(crate) pixels: Vec<u32>,
    pub(crate) largeur: usize,
    pub(crate) hauteur: usize,
    pub(crate) x: usize,
    pub(crate) y: usize,
}

struct Frame {
    #[cfg(feature = "p10-virgl-experimental")]
    couches: Vec<Couche>,
    pixels: Vec<u32>,
    largeur: usize,
    hauteur: usize,
    serie: u64,
}

struct Etat {
    largeur: usize,
    hauteur: usize,
    libres: Vec<Vec<u32>>,
    pret: Option<Frame>,
    #[cfg(feature = "p10-virgl-experimental")]
    couches_en_cours: Vec<Couche>,
    #[cfg(feature = "p10-virgl-experimental")]
    couches_libres: Vec<Vec<u32>>,
    actif: bool,
    serie: u64,
    soumissions: u64,
    presentees: u64,
    remplacees: u64,
    saturees: u64,
    echecs: u64,
    attente_gpu_ns: u64,
}

static ETAT: SpinLock<Option<Etat>> = SpinLock::new(None);
static TRAVAILLEUR: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "p10-virgl-experimental")]
static CLIENTS_AUTORISES: AtomicBool = AtomicBool::new(false);

// Rend l'optimisation GPU admissible seulement pour une fenetre non masquee.
#[cfg(feature = "p10-virgl-experimental")]
pub fn autorise_client(autorise: bool) -> bool {
    let etait_autorise = CLIENTS_AUTORISES.swap(autorise, Ordering::AcqRel);
    if !autorise {
        // Une capture partielle ne doit pas etre interpretee comme une scene
        // complete lors d'un retour CPU (menu/occlusion/cursor/erreur GPU).
        if let Some(e) = ETAT.lock().as_mut() {
            while let Some(c) = e.couches_en_cours.pop() {
                e.couches_libres.push(c.pixels);
            }
        }
    }
    etait_autorise
}

// 'Some(true)' signifie: texture capturee ET backend GPU deja valide lors
// d'une presentation precedente. L'appelant peut omettre le memcpy CPU.
// 'Some(false)' fait une premiere presentation GPU comparative tout en
// conservant un rendu CPU de secours pour la premiere image.
#[cfg(feature = "p10-virgl-experimental")]
pub fn prepare_client(
    largeur: usize, hauteur: usize, x: usize, y: usize,
    mut ligne: impl FnMut(usize, &mut [u32]),
) -> Option<bool> {
    if !CLIENTS_AUTORISES.load(Ordering::Acquire)
        || !crate::drivers::virtio_gpu::p10_client_gpu_possible()
        || largeur == 0 || hauteur == 0 || largeur > 4096 || hauteur > 2160 {
        return None;
    }
    let n = largeur.checked_mul(hauteur)?;
    if n > 1280 * 720 { return None; }
    let ok = crate::drivers::virtio_gpu::p10_client_gpu_verifie();
    let mut pixels = {
        let mut garde = ETAT.lock();
        let e = garde.as_mut()?;
        if !e.actif || e.couches_en_cours.len() >= MAX_COUCHES
            || x.checked_add(largeur).is_none_or(|v| v > e.largeur)
            || y.checked_add(hauteur).is_none_or(|v| v > e.hauteur) {
            return None;
        }
        for autre in &e.couches_en_cours {
            if autre.x == x && autre.y == y
                && autre.largeur == largeur && autre.hauteur == hauteur {
                // Meme surface redessinee dans plusieurs regions de damage:
                // le snapshot complet a deja ete capture dans cette trame.
                return Some(ok);
            }
            if x < autre.x + autre.largeur && autre.x < x + largeur
                && y < autre.y + autre.hauteur && autre.y < y + hauteur {
                return None; // pas d'occlusion GPU sur cette premiere version
            }
        }
        e.couches_libres.pop().unwrap_or_default()
    };
    if pixels.capacity() < n {
        pixels.try_reserve_exact(n.saturating_sub(pixels.len())).ok()?;
    }
    pixels.resize(n, 0);
    for yy in 0..hauteur {
        ligne(yy, &mut pixels[yy * largeur..(yy + 1) * largeur]);
    }
    let mut garde = ETAT.lock();
    let e = garde.as_mut()?;
    if !e.actif || e.couches_en_cours.len() >= MAX_COUCHES {
        e.couches_libres.push(pixels);
        return None;
    }
    e.couches_en_cours.push(Couche { pixels, largeur, hauteur, x, y });
    Some(ok)
}

fn tampon(pixels: usize) -> Option<Vec<u32>> {
    let mut v = Vec::new();
    v.try_reserve_exact(pixels).ok()?;
    v.resize(pixels, 0);
    Some(v)
}

/// Ouvre un service GPU pour le scanout actif. En cas d'echec, le chemin
/// virtio 2D synchrone historique reste disponible.
pub fn initialise(largeur: usize, hauteur: usize) -> bool {
    let Some(n) = largeur.checked_mul(hauteur) else { return false };
    if n == 0 || largeur > 4096 || hauteur > 2160 { return false; }

    {
        let mut g = ETAT.lock();
        if let Some(e) = g.as_mut() {
            if e.largeur != largeur || e.hauteur != hauteur { return false; }
            e.actif = true;
            return true;
        }
    }
    // Preallocation HORS spinlock : deux snapshots complets au plus.
    let Some(a) = tampon(n) else { return false };
    let Some(b) = tampon(n) else { return false };
    let mut libres = Vec::new();
    if libres.try_reserve_exact(2).is_err() { return false; }
    libres.push(a);
    libres.push(b);
    {
        let mut g = ETAT.lock();
        if g.is_some() { return false; }
        *g = Some(Etat {
            largeur, hauteur, libres, pret: None, actif: true,
            #[cfg(feature = "p10-virgl-experimental")]
            couches_en_cours: Vec::new(),
            #[cfg(feature = "p10-virgl-experimental")]
            couches_libres: Vec::new(),
            serie: 0, soumissions: 0, presentees: 0,
            remplacees: 0, saturees: 0, echecs: 0, attente_gpu_ns: 0,
        });
    }
    if !crate::kernel::task::spawn_noyau(fil_gpu, "p10-gpu-present") {
        *ETAT.lock() = None;
        return false;
    }
    TRAVAILLEUR.store(true, Ordering::Release);
    true
}

pub fn actif() -> bool {
    TRAVAILLEUR.load(Ordering::Acquire)
        && ETAT.lock().as_ref().is_some_and(|e| e.actif)
}

/// Appelé sous la barriere BASCULE lors de la sortie du mode graphique.
pub fn arrete() {
    if let Some(e) = ETAT.lock().as_mut() {
        e.actif = false;
        #[cfg(feature = "p10-virgl-experimental")]
        {
            while let Some(c) = e.couches_en_cours.pop() {
                e.couches_libres.push(c.pixels);
            }
        }
        if let Some(p) = e.pret.take() {
            #[cfg(feature = "p10-virgl-experimental")]
            for c in p.couches { e.couches_libres.push(c.pixels); }
            e.libres.push(p.pixels);
        }
    }
}

/// Soumission du framebuffer CPU UNIQUEMENT par le fil compositeur.
/// Copier une image pleine garantit qu'un remplacement de trame ne laisse
/// aucune zone obsolète dans les buffers de l'hote.
pub fn publie(source: &[u32], largeur: usize, hauteur: usize) -> bool {
    if !TRAVAILLEUR.load(Ordering::Acquire) { return false; }
    if !super::p10_surface_contract::trame_valide(largeur, hauteur, source.len()) {
        return false;
    }
    let mut buffer = {
        let mut garde = ETAT.lock();
        let Some(e) = garde.as_mut() else { return false };
        if !e.actif || (e.largeur, e.hauteur) != (largeur, hauteur) { return false; }
        if let Some(v) = e.libres.pop() {
            v
        } else if let Some(ancienne) = e.pret.take() {
            e.remplacees += 1;
            #[cfg(feature = "p10-virgl-experimental")]
            for c in ancienne.couches { e.couches_libres.push(c.pixels); }
            ancienne.pixels
        } else {
            e.saturees += 1;
            #[cfg(feature = "p10-virgl-experimental")]
            while let Some(c) = e.couches_en_cours.pop() {
                e.couches_libres.push(c.pixels);
            }
            return false;
        }
    };
    buffer.copy_from_slice(source);
    let mut garde = ETAT.lock();
    let Some(e) = garde.as_mut() else { return false };
    if !e.actif { e.libres.push(buffer); return false; }
    e.serie = e.serie.wrapping_add(1);
    let serie = e.serie;
    #[cfg(feature = "p10-virgl-experimental")]
    let couches = core::mem::take(&mut e.couches_en_cours);
    let nouveau = Frame {
        #[cfg(feature = "p10-virgl-experimental")]
        couches,
        pixels: buffer, largeur, hauteur, serie,
    };
    if let Some(ancien) = e.pret.replace(nouveau) {
        e.remplacees += 1;
        #[cfg(feature = "p10-virgl-experimental")]
        for c in ancien.couches { e.couches_libres.push(c.pixels); }
        e.libres.push(ancien.pixels);
    }
    e.soumissions += 1;
    true
}

fn fil_gpu() -> ! {
    loop {
        let frame = {
            let mut garde = ETAT.lock();
            garde.as_mut().and_then(|e| if e.actif { e.pret.take() } else { None })
        };
        if let Some(mut frame) = frame {
            let debut = crate::kernel::timer::monotonic_ns();
            #[cfg(feature = "p10-virgl-experimental")]
            let mut ok = if !frame.couches.is_empty() {
                crate::drivers::gfx::p10_publie_avec_clients(
                    &frame.pixels, frame.largeur, frame.hauteur, &frame.couches,
                )
            } else {
                crate::drivers::gfx::p10_publie_physiquement(
                    &frame.pixels, frame.largeur, frame.hauteur,
                )
            };
            #[cfg(not(feature = "p10-virgl-experimental"))]
            let mut ok = crate::drivers::gfx::p10_publie_physiquement(
                &frame.pixels, frame.largeur, frame.hauteur,
            );
            // Si le GPU refuse une image, restaurer TOUTES les surfaces
            // dans le snapshot CPU AVANT le seul repli 2D.
            #[cfg(feature = "p10-virgl-experimental")]
            if !ok && !frame.couches.is_empty() {
                for c in &frame.couches {
                    for yy in 0..c.hauteur {
                        let dst = (c.y + yy) * frame.largeur + c.x;
                        let src = yy * c.largeur;
                        frame.pixels[dst..dst + c.largeur]
                            .copy_from_slice(&c.pixels[src..src + c.largeur]);
                    }
                }
                ok = crate::drivers::gfx::p10_publie_physiquement(
                    &frame.pixels, frame.largeur, frame.hauteur,
                );
            }
            let delai = crate::kernel::timer::monotonic_ns().saturating_sub(debut);
            let mut rapport = None;
            {
                let mut garde = ETAT.lock();
                if let Some(e) = garde.as_mut() {
                    if ok { e.presentees += 1; } else { e.echecs += 1; }
                    e.attente_gpu_ns = e.attente_gpu_ns.saturating_add(delai);
                    let n = e.presentees + e.echecs;
                    if n != 0 && n % 64 == 0 {
                        rapport = Some((e.soumissions, e.presentees, e.remplacees,
                                        e.saturees, e.echecs, e.attente_gpu_ns / n / 1_000));
                    }
                    #[cfg(feature = "p10-virgl-experimental")]
                    for c in frame.couches.drain(..) { e.couches_libres.push(c.pixels); }
                    e.libres.push(frame.pixels);
                }
            }
            if let Some((soumises, presentees, remplacees, saturation, echecs, moyen_us)) = rapport {
                crate::serial_println!(
                    "P10_GPU_QUEUE serie={} soumises={} presentees={} remplacees={} saturation={} echecs={} us_moyen_gpu={}",
                    frame.serie, soumises, presentees, remplacees, saturation, echecs, moyen_us,
                );
            }
        } else {
            crate::kernel::task::sleep_ticks(1);
        }
    }
}

/// Etat lisible pour l'audit de performance et le banc A/B.
pub fn stats() -> (u64, u64, u64, u64, u64, u64) {
    let g = ETAT.lock();
    let Some(e) = g.as_ref() else { return (0, 0, 0, 0, 0, 0) };
    (e.soumissions, e.presentees, e.remplacees, e.saturees,
     e.echecs, e.attente_gpu_ns)
}
