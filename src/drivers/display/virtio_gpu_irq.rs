//! Achevement des commandes virtio-gpu par interruption : ce qui se decide
//! sans toucher au materiel (BOUCHAUD_VIRTIO_GPU_IRQ_V1, P10+.1).
//!
//! # Ce que cela remplace
//!
//! L'attente d'une reponse de la file de controle SCRUTAIT l'anneau utilise :
//! 50 us de rotation, puis une cession du processeur entre deux lectures.
//! Mesure (Ladybird sur virtio-gpu, run 38645396730, c7018e56) : 202
//! presentations, 1,07 ms chacune, 7 789 cessions -- trente-huit passages
//! dans l'ordonnanceur par presentation, pour rien quand personne d'autre
//! n'est pret.
//!
//! # Le modele
//!
//! Le peripherique signale l'avance de l'anneau utilise par MSI-X (vecteur
//! LAPIC dedie). Le gestionnaire d'interruption n'execute AUCUNE commande et
//! ne prend AUCUN verrou : il compte, compare `used.idx` a ce qu'il a deja
//! vu (une interruption sans progres est comptee parasite) et reveille la
//! file d'attente des achevements. L'attendant :
//!
//!   1. tourne au plus `rotation_ns` (sous KVM la reponse arrive en quelques
//!      dizaines de microsecondes : s'endormir couterait plus cher) ;
//!   2. puis DORT sur la file, jusqu'a la plus proche de deux bornes :
//!      l'echeance de la commande, ou `relecture_ns` plus tard. La seconde
//!      borne est le filet d'une interruption PERDUE : la reponse est relue
//!      au plus tard toutes les `relecture_ns`, et l'achevement vu ainsi est
//!      COMPTE (`vus_par_relecture`) -- jamais tu ;
//!   3. chaque reveil relit l'anneau : un reveil sans progres (interruption
//!      parasite, ou destinee a une commande precedente) n'acheve rien.
//!
//! La prise du ticket de la file d'attente PRECEDE la derniere relecture
//! (`Dormeur::dors`) : une interruption arrivee entre les deux n'est pas
//! perdue.
//!
//! # Ou l'on ne dort pas
//!
//! Hors de toute tache (sonde et autotest au demarrage), interruptions
//! masquees, verrou tournant tenu, ou interruption non armee : la scrutation
//! bornee d'avant reste le seul chemin (`strategie`).
//!
//! Module pur : `tools/display/test_virtio_gpu_irq.rs`.

/// Le vecteur LAPIC des achevements virtio-gpu. Hors de la plage du 8259
/// (0x20..0x2F) et des IPI (0xF1..0xF3, 0xFF).
pub const VECTEUR: u8 = 0x50;

/// `VIRTIO_MSI_NO_VECTOR` : aucun vecteur (ce que le peripherique rend quand
/// il refuse une association).
pub const AUCUN_VECTEUR: u16 = 0xFFFF;

/// Bits du registre de controle MSI-X (PCI 3.0, 6.8.2.3).
pub const MSIX_ACTIVE: u16 = 1 << 15;
pub const MSIX_MASQUE_FONCTION: u16 = 1 << 14;

/// Ou se trouve la table MSI-X : le BAR (BIR, trois bits bas) et le decalage
/// (le reste, aligne sur 8 octets).
pub const fn emplacement_table(bir_et_decalage: u32) -> (u8, u32) {
    ((bir_et_decalage & 0x7) as u8, bir_et_decalage & !0x7)
}

/// Le message MSI d'une interruption fixe, en front, vers un LAPIC.
///
/// Adresse : 0xFEE0_0000 | destination << 12 (mode physique, sans
/// redirection) ; donnee : le vecteur (livraison fixe, front). Un identifiant
/// d'APIC au-dela de 255 ne tient pas dans ce format (il faudrait le
/// remappage d'interruptions) : `None`, et le pilote reste en scrutation.
pub const fn message(apic: u32, vecteur: u8) -> Option<(u32, u32, u32)> {
    if apic > 0xFF || vecteur < 0x20 {
        return None;
    }
    Some((0xFEE0_0000 | (apic << 12), 0, vecteur as u32))
}

/// Comment attendre une reponse, ici.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategie {
    /// Scrutation bornee (rotation, puis cession si permise).
    Scrute,
    /// Rotation courte, puis sommeil sur la file des achevements.
    Dort,
}

/// `cedable` : une tache courante, interruptions ouvertes, aucun verrou
/// tournant ; `armee` : le peripherique a accepte le vecteur ; `voulue` :
/// la politique (commande `gpu-irq`).
pub const fn strategie(cedable: bool, armee: bool, voulue: bool) -> Strategie {
    if cedable && armee && voulue {
        Strategie::Dort
    } else {
        Strategie::Scrute
    }
}

/// Une interruption a-t-elle apporte du nouveau ? `vu` : le dernier
/// `used.idx` vu par le gestionnaire ; `idx` : celui qu'il lit maintenant.
pub const fn progres(vu: u16, idx: u16) -> bool {
    vu != idx
}

/// Ce que l'attente a vu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bilan {
    /// La reponse est arrivee avant l'echeance.
    pub reussi: bool,
    /// Sommeils pris sur la file des achevements.
    pub sommeils: u32,
    /// Reveils par interruption sans reponse (parasite, ou d'une autre
    /// commande).
    pub reveils_sans_progres: u32,
    /// La reponse a ete vue par la relecture de secours, sans interruption :
    /// une interruption a ete perdue (ou retardee au-dela de `relecture_ns`).
    pub vu_par_relecture: bool,
    /// La reponse est arrivee pendant la rotation (aucun sommeil).
    pub vu_en_rotation: bool,
}

/// Comment un sommeil s'est termine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reveil {
    /// La reponse etait la apres la prise du ticket : pas de sommeil.
    DejaPret,
    /// Une interruption a reveille (avec ou sans reponse).
    Interruption,
    /// La borne est atteinte sans interruption.
    Borne,
}

/// Ce que l'attente demande au monde : l'heure, et dormir.
pub trait Dormeur {
    /// L'heure monotone (ns).
    fn maintenant(&mut self) -> u64;
    /// Prend un ticket de la file des achevements, relit `pret`, et dort
    /// jusqu'a `jusqu_a` (ns) au plus si la reponse n'est pas encore la.
    fn dors(&mut self, jusqu_a: u64, pret: &mut dyn FnMut() -> bool) -> Reveil;
}

/// Attend que `pret` devienne vrai, au plus jusqu'a `echeance_ns`.
pub fn attend(
    pret: &mut dyn FnMut() -> bool,
    monde: &mut dyn Dormeur,
    echeance_ns: u64,
    rotation_ns: u64,
    relecture_ns: u64,
) -> Bilan {
    let mut b = Bilan::default();
    let debut = monde.maintenant();
    loop {
        if pret() {
            b.reussi = true;
            b.vu_en_rotation = true;
            return b;
        }
        let t = monde.maintenant();
        if t >= echeance_ns {
            return b;
        }
        if t.saturating_sub(debut) >= rotation_ns {
            break;
        }
        core::hint::spin_loop();
    }
    loop {
        let t = monde.maintenant();
        if t >= echeance_ns {
            // Derniere lecture : une reponse arrivee a l'instant compte.
            b.reussi = pret();
            b.vu_par_relecture = b.reussi;
            return b;
        }
        let jusqu_a = core::cmp::min(echeance_ns, t.saturating_add(relecture_ns.max(1)));
        let reveil = monde.dors(jusqu_a, pret);
        if reveil != Reveil::DejaPret {
            b.sommeils += 1;
        }
        if pret() {
            b.reussi = true;
            // Vue a la borne, sans interruption : une interruption perdue,
            // ou plus lente que la relecture.
            b.vu_par_relecture = reveil == Reveil::Borne;
            return b;
        }
        if reveil == Reveil::Interruption {
            b.reveils_sans_progres += 1;
        }
    }
}
