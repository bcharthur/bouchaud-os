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

// BOUCHAUD_P10_GPU_APP_LAYERS_20261109
#[cfg(feature = "p10-virgl-experimental")]
struct Couche {
    pixels: Vec<u32>,
    largeur: usize,
    hauteur: usize,
    x: usize,
    y: usize,
}

struct Frame {
    #[cfg(feature = "p10-virgl-experimental")]
    couche: Option<Couche>,
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
    couche_en_cours: Option<Couche>,
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
static CLIENT_UNIQUE_AUTORISE: AtomicBool = AtomicBool::new(false);

// Rend l'optimisation GPU admissible seulement pour une fenetre non masquee.
#[cfg(feature = "p10-virgl-experimental")]
pub fn autorise_client(autorise: bool) {
    CLIENT_UNIQUE_AUTORISE.store(autorise, Ordering::Release);
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
    if !CLIENT_UNIQUE_AUTORISE.load(Ordering::Acquire)
        || !crate::drivers::virtio_gpu::p10_client_gpu_possible()
        || largeur == 0 || hauteur == 0 || largeur > 4096 || hauteur > 2160 {
        return None;
    }
    let n = largeur.checked_mul(hauteur)?;
    if n > 1280 * 720 { return None; }
    let mut pixels = {
        let mut etat = ETAT.lock();
        let e = etat.as_mut()?;
        if !e.actif || e.couche_en_cours.is_some()
            || x.checked_add(largeur).is_none_or(|v| v > e.largeur)
            || y.checked_add(hauteur).is_none_or(|v| v > e.hauteur) {
            return None;
        }
        e.couches_libres.pop().unwrap_or_default()
    };
    // Reutilise les pages de la premiere surface; pas de fautes Zero a
    // chaque trame du navigateur (P10 et C22).
    if pixels.capacity() < n {
        pixels.try_reserve_exact(n.saturating_sub(pixels.len())).ok()?;
    }
    pixels.resize(n, 0);
    for yy in 0..hauteur {
        ligne(yy, &mut pixels[yy * largeur..(yy + 1) * largeur]);
    }
    let accelere = crate::drivers::virtio_gpu::p10_client_gpu_verifie();
    let mut etat = ETAT.lock();
    let e = etat.as_mut()?;
    if !e.actif || e.couche_en_cours.is_some() { return None; }
    e.couche_en_cours = Some(Couche { pixels, largeur, hauteur, x, y });
    Some(accelere)
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
            couche_en_cours: None,
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
        { e.couche_en_cours = None; }
        if let Some(p) = e.pret.take() {
            #[cfg(feature = "p10-virgl-experimental")]
            if let Some(c) = p.couche { e.couches_libres.push(c.pixels); }
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
        let mut g = ETAT.lock();
        let Some(e) = g.as_mut() else { return false };
        if !e.actif || (e.largeur, e.hauteur) != (largeur, hauteur) {
            return false;
        }
        if let Some(v) = e.libres.pop() {
            v
        } else if let Some(ancienne) = e.pret.take() {
            e.remplacees += 1;
            #[cfg(feature = "p10-virgl-experimental")]
            if let Some(c) = ancienne.couche { e.couches_libres.push(c.pixels); }
            ancienne.pixels
        } else {
            e.saturees += 1;
            #[cfg(feature = "p10-virgl-experimental")]
            if let Some(c) = e.couche_en_cours.take() {
                e.couches_libres.push(c.pixels);
            }
            return false;
        }
    };
    buffer.copy_from_slice(source); // jamais sous un verrou de peripherique
    let mut g = ETAT.lock();
    let Some(e) = g.as_mut() else { return false };
    if !e.actif { e.libres.push(buffer); return false; }
    e.serie = e.serie.wrapping_add(1);
    let serie = e.serie;
    #[cfg(feature = "p10-virgl-experimental")]
    let couche = e.couche_en_cours.take();
    let nouveau = Frame {
        #[cfg(feature = "p10-virgl-experimental")]
        couche,
        pixels: buffer, largeur, hauteur, serie,
    };
    if let Some(ancien) = e.pret.replace(nouveau) {
        e.remplacees += 1;
        #[cfg(feature = "p10-virgl-experimental")]
        if let Some(c) = ancien.couche { e.couches_libres.push(c.pixels); }
        e.libres.push(ancien.pixels);
    }
    e.soumissions += 1;
    true
}

fn fil_gpu() -> ! {
    loop {
        let frame = {
            let mut g = ETAT.lock();
            g.as_mut().and_then(|e| if e.actif { e.pret.take() } else { None })
        };
        if let Some(mut frame) = frame {
            let debut = crate::kernel::timer::monotonic_ns();
            // BOUCHAUD_P10_ORIENTATION_ATOMIC_20261109
            // Ne plus presenter le fond 2D puis le contenu 3D: deux flush
            // successifs produisaient le clignotement observe en QEMU.
            // Une trame avec surface cliente execute une transaction unique
            // sous BASCULE: transfert 2D SANS flush, texture/BLIT GPU, puis
            // RESOURCE_FLUSH complet. Les autres restent sur le chemin 2D.
            #[cfg(feature = "p10-virgl-experimental")]
            let mut ok = if let Some(c) = frame.couche.as_ref() {
                crate::drivers::gfx::p10_publie_avec_client(
                    &frame.pixels, frame.largeur, frame.hauteur,
                    &c.pixels, c.largeur, c.hauteur, c.x, c.y,
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
            // La recuperation CPU se fait HORS des verrous GPU. Elle restaure
            // exactement les pixels de la fenetre lorsque le GPU refuse.
            #[cfg(feature = "p10-virgl-experimental")]
            let mut reutiliser: Option<Vec<u32>> = None;
            #[cfg(feature = "p10-virgl-experimental")]
            if let Some(c) = frame.couche.take() {
                if !ok {
                    for yy in 0..c.hauteur {
                        let dst = (c.y + yy) * frame.largeur + c.x;
                        let src = yy * c.largeur;
                        frame.pixels[dst..dst + c.largeur]
                            .copy_from_slice(&c.pixels[src..src + c.largeur]);
                    }
                    ok = crate::drivers::gfx::p10_publie_physiquement(
                        &frame.pixels, frame.largeur, frame.hauteur,
                    );
                }
                reutiliser = Some(c.pixels);
            }
            let delai = crate::kernel::timer::monotonic_ns().saturating_sub(debut);
            let mut rapport = None;
            {
                let mut g = ETAT.lock();
                if let Some(e) = g.as_mut() {
                    if ok { e.presentees += 1; } else { e.echecs += 1; }
                    e.attente_gpu_ns = e.attente_gpu_ns.saturating_add(delai);
                    let n = e.presentees + e.echecs;
                    if n != 0 && n % 64 == 0 {
                        rapport = Some((e.soumissions, e.presentees, e.remplacees,
                                        e.saturees, e.echecs,
                                        e.attente_gpu_ns / n / 1_000));
                    }
                    // Recycle les images, evite allocations/zero-faults C22.
                    #[cfg(feature = "p10-virgl-experimental")]
                    if let Some(pixels) = reutiliser.take() {
                        e.couches_libres.push(pixels);
                    }
                    e.libres.push(frame.pixels);
                }
            }
            // Une console serie lente ne doit jamais retenir le spinlock.
            if let Some((soumises, presentees, remplacees, saturation, echecs, moyen_us)) = rapport {
                crate::serial_println!(
                    "P10_GPU_QUEUE serie={} soumises={} presentees={} remplacees={} saturation={} echecs={} us_moyen_gpu={}",
                    frame.serie, soumises, presentees, remplacees,
                    saturation, echecs, moyen_us,
                );
            }
        } else {
            // Attente courte et PREEMPTIBLE: pas de spinlock conserve.
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
