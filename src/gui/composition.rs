//! BOUCHAUD_GPU_COMPOSITION_V2, cote bureau : QUELLES fenetres le GPU compose
//! a cette trame, et lesquels de leurs pixels.
//!
//! Regle unique, tiree du plan de scene : tout calque dessine APRES une
//! fenetre (fenetres au-dessus, fantome de fermeture, menu, centre, dock,
//! infobulle, curseur) la recouvre. Le GPU pose la zone du client MOINS ces
//! calques ; le processeur peint, au pixel, ce qui est dessous (une ombre sur
//! le contenu, le curseur et les pixels du client sous lui). Ce qui ne se
//! decoupe pas proprement (plus de `MORCEAUX_MAX` morceaux, fenetre en
//! animation, client qui n'a pas encore peint) reste au processeur.
//!
//! Le plan de la trame en cours est publie ici pour `widgets::compose_client`,
//! qui ne recopie plus que les pixels du client hors des morceaux GPU.

use crate::drivers::composition_gpu::{self as cg, R, MORCEAUX_MAX};
use crate::gui::protocole::Rect;
use crate::gui::scene::{Calque, Element};
use crate::gui::window::Win;
use crate::kernel::sync::SpinLock;

pub const FENETRES_MAX: usize = 8;

#[derive(Clone, Copy)]
pub struct FenetreGpu {
    pub id: u64,
    /// Indice dans `wins` a la trame du plan.
    pub index: usize,
    /// Zone du client a l'ecran, bornee a ce qu'il a peint.
    pub zone: R,
    pub visibles: [R; MORCEAUX_MAX],
    pub n: usize,
}

/// Ce qui reste VISIBLE d'une fenetre cliente une fois retirees les zones
/// OPAQUES des calques superieurs : un degat du client hors de ces morceaux
/// ne se voit pas, ni le processeur ni le GPU n'ont a le traiter.
#[derive(Clone, Copy)]
pub struct Masque {
    pub id: u64,
    pub morceaux: [R; MORCEAUX_MAX],
    pub n: usize,
}

#[derive(Clone, Copy)]
pub struct Plan {
    pub fenetres: [Option<FenetreGpu>; FENETRES_MAX],
    pub masques: [Option<Masque>; FENETRES_MAX],
}

impl Plan {
    pub const fn vide() -> Plan {
        Plan { fenetres: [None; FENETRES_MAX], masques: [None; FENETRES_MAX] }
    }
    pub fn de(&self, id: u64) -> Option<&FenetreGpu> {
        self.fenetres.iter().flatten().find(|f| f.id == id)
    }
    pub fn nombre(&self) -> usize {
        self.fenetres.iter().flatten().count()
    }
}

static COURANT: SpinLock<Plan> = SpinLock::new(Plan::vide());

use core::sync::atomic::{AtomicU64, Ordering};
/// Temps du fil du bureau par trame : DESSIN (processeur : bureau et pixels
/// clients recopies) et PRESENTATION (commandes au GPU, attentes comprises).
static NS_DESSIN: AtomicU64 = AtomicU64::new(0);
static NS_PRESENTATION: AtomicU64 = AtomicU64::new(0);
static TRAMES_BUREAU: AtomicU64 = AtomicU64::new(0);

pub fn note_trame(dessin_ns: u64, presentation_ns: u64) {
    NS_DESSIN.fetch_add(dessin_ns, Ordering::Relaxed);
    NS_PRESENTATION.fetch_add(presentation_ns, Ordering::Relaxed);
    TRAMES_BUREAU.fetch_add(1, Ordering::Relaxed);
}

fn r(rect: Rect) -> R {
    R::neuf(rect.x, rect.y, rect.largeur.min(i32::MAX as u32) as i32, rect.hauteur.min(i32::MAX as u32) as i32)
}

/// La zone composable du client de `w` a l'ecran (vide sans client actif).
pub fn zone_client(w: &Win) -> R {
    let Some(client) = w.app.client() else { return R::default() };
    if client.etat != crate::gui::client::Etat::Actif {
        return R::default();
    }
    let z = crate::gui::window::zone_utile(w);
    let (pl, ph) = client.composable();
    let l = (z.largeur.max(0) as usize).min(pl).min(i32::MAX as usize) as i32;
    let h = (z.hauteur.max(0) as usize).min(ph).min(i32::MAX as usize) as i32;
    R::neuf(z.x, z.y, l, h)
}

/// `planifie` sur le TAS (la pile du bureau est comptee), ou un plan vide si
/// aucune trame ne peut s'ouvrir.
#[inline(never)]
pub fn planifie_si(possible: bool, wins: &[Win], calques: &[Calque], ecran: R) -> alloc::boxed::Box<Plan> {
    if possible {
        alloc::boxed::Box::new(planifie(wins, calques, ecran))
    } else {
        alloc::boxed::Box::new(Plan::vide())
    }
}

/// Le plan de la trame : `calques` est le plan de scene de cette trame.
pub fn planifie(wins: &[Win], calques: &[Calque], ecran: R) -> Plan {
    let mut plan = Plan::vide();
    let gpu = crate::drivers::virtio_gpu::composition_gpu_disponible();
    let mut k = 0usize;
    let mut km = 0usize;
    for (i, calque) in calques.iter().enumerate() {
        let Element::Fenetre(index) = calque.element else { continue };
        let Some(w) = wins.get(index) else { continue };
        if w.min || crate::gui::atelier::effets::ouverture(w.id.get()).is_some() {
            continue;
        }
        let zone = zone_client(w).inter(&ecran);
        if zone.vide() {
            continue;
        }
        // Le masque : la zone moins les parties OPAQUES de ce qui est dessus.
        if km < FENETRES_MAX {
            let mut opaques = [R::default(); 32];
            let mut t = 0usize;
            for dessus in &calques[i + 1..] {
                if let Some(o) = dessus.opaque_sur {
                    let o = r(o);
                    if o.touche(&zone) && t < opaques.len() {
                        opaques[t] = o;
                        t += 1;
                    }
                }
            }
            let mut morceaux = [R::default(); MORCEAUX_MAX];
            if let Some(n) = cg::soustrait(zone, &opaques[..t], &mut morceaux) {
                plan.masques[km] = Some(Masque { id: w.id.get(), morceaux, n });
                km += 1;
            }
        }
        if !gpu {
            continue;
        }
        let mut trous = [R::default(); 32];
        let mut t = 0usize;
        let mut trop = false;
        for dessus in &calques[i + 1..] {
            let b = r(dessus.bornes_dessin);
            if !b.touche(&zone) {
                continue;
            }
            if t == trous.len() {
                trop = true;
                break;
            }
            trous[t] = b;
            t += 1;
        }
        if trop {
            continue;
        }
        let mut visibles = [R::default(); MORCEAUX_MAX];
        let Some(n) = cg::soustrait(zone, &trous[..t], &mut visibles) else { continue };
        if n == 0 {
            continue;
        }
        if k == FENETRES_MAX {
            break;
        }
        plan.fenetres[k] = Some(FenetreGpu { id: w.id.get(), index, zone, visibles, n });
        k += 1;
    }
    plan
}

/// Publie le plan de la trame en cours (lu par `compose_client`).
pub fn publie(plan: &Plan) {
    *COURANT.lock() = *plan;
}

/// Les morceaux poses par le GPU pour la fenetre `id` a cette trame.
pub fn morceaux_gpu(id: u64) -> Option<([R; MORCEAUX_MAX], usize)> {
    COURANT.lock().de(id).map(|f| (f.visibles, f.n))
}

/// Le releve `[GPU-COMPOSITION]` (une ligne par releve du bureau). Lu par
/// `tools/ci/run_gpu_composition.sh` pour l'A/B processeur / GPU.
/// `trames_clients` : trames recues des clients pendant la periode (le
/// compteur des clients est remis a zero a chaque releve).
pub fn releve(periode_ms: u64, trames_clients: u64) {
    static PRECEDENT: SpinLock<[u64; 9]> = SpinLock::new([0; 9]);
    let (trames, blits, poses, montes, textures, _ns, dispo) =
        crate::drivers::virtio_gpu::compteurs_composition_gpu();
    let cpu = crate::gui::widgets::pixels_client_copies_par_le_processeur();
    let maintenant = [trames, blits, poses, montes, cpu, NS_DESSIN.load(Ordering::Relaxed),
        NS_PRESENTATION.load(Ordering::Relaxed), TRAMES_BUREAU.load(Ordering::Relaxed), trames_clients];
    let avant = core::mem::replace(&mut *PRECEDENT.lock(), maintenant);
    let d = |i: usize| maintenant[i].saturating_sub(avant[i]);
    let tb = d(7).max(1);
    {
        let p = COURANT.lock();
        for m in p.masques.iter().flatten() {
            let visible: u64 = m.morceaux[..m.n].iter().map(|r| r.aire()).sum();
            crate::serial_println!("[GPU-COMPOSITION-MASQUE] fenetre={} morceaux_visibles={} px_visibles={} gpu={}",
                m.id, m.n, visible, p.de(m.id).is_some() as u8);
        }
    }
    crate::serial_println!(
        "[GPU-COMPOSITION] disponible={} periode_ms={} trames_bureau={} trames_clients={} trames_gpu={} blits={} px_poses_gpu={} px_montes_hote={} px_client_copies_cpu={} us_dessin_cpu_par_trame={} us_presentation_par_trame={} textures={} fenetres_gpu={} cpu_pct={}",
        dispo as u8, periode_ms, d(7), trames_clients, d(0), d(1), d(2), d(3), d(4),
        d(5) / tb / 1000, d(6) / tb / 1000, textures, COURANT.lock().nombre(),
        crate::kernel::timer::cpu_load_pct());
}

/// Le degat `ecran` d'un client : les rectangles que le PROCESSEUR doit
/// repeindre (hors morceaux poses par le GPU, hors zones couvertes par un
/// calque OPAQUE superieur), ajoutes a `sortie` ; rend s'il reste du travail
/// au GPU. Plusieurs rectangles, pas leur englobant : deux bandes fines aux
/// bords d'une fenetre cachee ne doivent pas repeindre toute la fenetre.
#[inline(never)]
pub fn route_degat(id: u64, ecran: Rect, sortie: &mut alloc::vec::Vec<Rect>) -> bool {
    let (gpu, masque) = {
        let p = COURANT.lock();
        (p.de(id).map(|f| (f.visibles, f.n)), p.masques.iter().flatten().find(|m| m.id == id).copied())
    };
    let e = R::neuf(ecran.x, ecran.y, ecran.largeur.min(i32::MAX as u32) as i32,
        ecran.hauteur.min(i32::MAX as u32) as i32);
    let mut reste = [R::default(); MORCEAUX_MAX * 2];
    let k = match gpu {
        Some((g, n)) => cg::soustrait(e, &g[..n], &mut reste),
        None => { reste[0] = e; Some(1) }
    };
    let Some(k) = k else {
        sortie.push(ecran);
        return gpu.is_some();
    };
    for m in &reste[..k] {
        match &masque {
            // Seul ce qui n'est pas sous un calque opaque se voit.
            Some(mq) => for v in &mq.morceaux[..mq.n] {
                let x = m.inter(v);
                if !x.vide() {
                    sortie.push(Rect::neuf(x.x, x.y, x.l as u32, x.h as u32));
                }
            },
            None => sortie.push(Rect::neuf(m.x, m.y, m.l as u32, m.h as u32)),
        }
    }
    gpu.is_some()
}
