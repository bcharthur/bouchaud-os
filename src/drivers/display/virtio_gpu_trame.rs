//! Une trame, une soumission (BOUCHAUD_VIRTIO_GPU_TRAME_V1, P10+.3a).
//!
//! # Ce qui motive
//!
//! Le compositeur presente chaque region endommagee d'une trame par
//! `present_rect` : sous virtio-gpu, DEUX commandes synchrones par region
//! (`TRANSFER_TO_HOST_2D`, puis `RESOURCE_FLUSH` apres sa reponse). Une trame
//! a cinq regions coutait dix allers-retours, dont cinq flushs alors qu'un
//! seul -- sur l'union -- suffit a publier la trame entiere.
//!
//! # Le plan
//!
//! `planifie` rend, pour les regions d'une trame :
//!
//!   * les TRANSFERTS : les regions, fusionnees deux a deux tant que le
//!     surcout en pixels d'une fusion (l'aire ajoutee) ne depasse pas le cout
//!     d'une commande, exprime en pixels (`cout_commande_px`) ; au plus
//!     `TRANSFERTS_MAX` (au-dela, les paires les moins couteuses fusionnent) ;
//!   * UN flush, sur l'union de la trame.
//!
//! Aucun pixel endommage n'est oublie (chaque region est couverte par un
//! transfert) ; le flush ne part qu'apres la reponse de TOUS les transferts
//! (l'ordre reste celui de BARRIERE_V1, une commande en vol).
//!
//! Module pur : `tools/display/test_virtio_gpu_trame.rs`.

use super::virtio_gpu_tampons::Rect;

/// Au-dela, les transferts d'une trame sont fusionnes.
pub const TRANSFERTS_MAX: usize = 8;

/// Le plan de soumission d'une trame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub transferts: [Rect; TRANSFERTS_MAX],
    pub nb: usize,
    pub flush: Rect,
}

impl Plan {
    pub fn transferts(&self) -> &[Rect] {
        &self.transferts[..self.nb]
    }

    /// Commandes envoyees : les transferts, plus le flush (aucune si vide).
    pub fn commandes(&self) -> usize {
        if self.nb == 0 { 0 } else { self.nb + 1 }
    }

    /// Pixels transferes (les fusions en ajoutent).
    pub fn pixels(&self) -> u64 {
        self.transferts().iter().map(|r| r.pixels()).sum()
    }
}

/// L'aire ajoutee en fusionnant deux rectangles.
fn surcout(a: Rect, b: Rect) -> u64 {
    a.union(b).pixels().saturating_sub(a.pixels() + b.pixels())
}

/// Le plan d'une trame. Les regions vides sont ignorees.
pub fn planifie(regions: &[Rect], cout_commande_px: u64) -> Plan {
    let mut plan = Plan { transferts: [Rect::vide(); TRANSFERTS_MAX], nb: 0, flush: Rect::vide() };
    for r in regions.iter().copied().filter(|r| !r.est_vide()) {
        plan.flush = plan.flush.union(r);
        if plan.nb < TRANSFERTS_MAX {
            plan.transferts[plan.nb] = r;
            plan.nb += 1;
        } else {
            // Plein : la region rejoint le transfert qu'elle agrandit le moins.
            let i = (0..plan.nb).min_by_key(|&i| surcout(plan.transferts[i], r)).unwrap_or(0);
            plan.transferts[i] = plan.transferts[i].union(r);
        }
        // Fusionne tant qu'une paire coute moins qu'une commande.
        loop {
            let mut meilleure: Option<(usize, usize, u64)> = None;
            for i in 0..plan.nb {
                for j in i + 1..plan.nb {
                    let c = surcout(plan.transferts[i], plan.transferts[j]);
                    if c <= cout_commande_px && meilleure.map(|(_, _, m)| c < m).unwrap_or(true) {
                        meilleure = Some((i, j, c));
                    }
                }
            }
            let Some((i, j, _)) = meilleure else { break };
            plan.transferts[i] = plan.transferts[i].union(plan.transferts[j]);
            plan.transferts[j] = plan.transferts[plan.nb - 1];
            plan.nb -= 1;
        }
    }
    plan
}

/// Chaque region endommagee est-elle couverte par un transfert du plan ?
pub fn couvre(plan: &Plan, regions: &[Rect]) -> bool {
    regions.iter().filter(|r| !r.est_vide()).all(|r| {
        plan.transferts().iter().any(|t| {
            t.x <= r.x && t.y <= r.y && t.x + t.largeur >= r.x + r.largeur && t.y + t.hauteur >= r.y + r.hauteur
        })
    })
}
