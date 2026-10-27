//! BOUCHAUD_SECOURS_COEUR_MUET_V1 -- une tache prete, en file sur un coeur qui
//! ne commute plus, est relogee sur un coeur vivant. La POLITIQUE, pure.
//!
//! # Le defaut
//!
//! os-primitives KVM SMP4 (run 39667691144, 6e8bf33e), banc du verrou de
//! l'anneau tenu 3 s interruptions masquees : 600/600 lignes intactes, mais
//! `repli_securite=1` -- la console asynchrone s'est rabattue sur le chemin
//! synchrone (`synchrones=1582`). Le fil `console` avait dormi une
//! milliseconde (`jeton occupe`) ; ce sommeil a dure 3 001 297 us
//! (`fil_somme_jeton_pire_us`). Reveille, il avait ete publie sur SON coeur
//! -- le seul ou il ait jamais tourne -- et c'etait celui que le banc masquait.
//! Un fil noyau n'est jamais vole (`runnable_steal`) : trois coeurs vivants
//! a cote, personne ne pouvait le prendre. Le chien de garde, a 2 s, a pris ce
//! fil vivant pour un fil mort.
//!
//! # La regle
//!
//! Un coeur est MUET quand son occupant y est installe depuis plus de
//! `seuil` : interruptions masquees, processeur virtuel retire par l'hote,
//! boucle sans point cooperatif. Un coeur qui respecte son quantum (4 ms au
//! plus) ou ses points cooperatifs (1 ms) ne l'est jamais : une tache en file
//! derriere lui le fait ceder bien avant. Une tache prete depuis plus de
//! `seuil`, en file sur un coeur muet, peut etre relogee -- sur un coeur
//! permis, vivant, de preference au repos, sinon le moins charge.
//!
//! Hors d'un coeur muet, rien ne change : l'equilibrage ordinaire, et sa
//! regle de ne jamais voler un fil noyau, restent ceux d'avant. Le secours
//! ne remplace pas le verdict d'un chien de garde : un fil qui ne progresse
//! plus, meme reloge, reste un fil muet.
//!
//! # P13_REVEIL_COEUR_MUET_V2 -- l'ordonnanceur l'appelle lui-meme
//!
//! Le secours n'etait appele que par le chien de garde de la console. Toute
//! autre tache prete en file sur un coeur muet attendait qu'un coeur VOISIN
//! se mette au repos pour la voler (`pick_next` ne vole que sur file vide).
//! KVM SMP2, run 40582472007 : le dormeur, publie sur le coeur zero que le
//! banc masquait, y est reste 193 a 420 ms (8 demarrages sur 30), le coeur un
//! jamais au repos -- il servait la console et ses voisins, battements a
//! l'heure, vol de l'hote nul. Desormais chaque coeur vivant, a son
//! battement, reloge les taches que cette regle designe
//! ([`coeurs_muets`] puis `secours_tache_coeur_muet`).
//!
//! Module pur : `tools/smp/test_secours_coeur_muet.rs` l'inclut tel quel.

/// Au-dela, un occupant qui n'a pas cede son coeur est anormal : cinq quanta
/// d'un AP, vingt points cooperatifs.
pub const SECOURS_SEUIL_NS: u64 = 20_000_000;

/// L'occupant du coeur y est-il installe depuis plus de `seuil_ns` ?
/// `occupant_depuis_ns == 0` : coeur sans tache (au repos), jamais muet.
pub fn coeur_muet(occupant_depuis_ns: u64, maintenant_ns: u64, seuil_ns: u64) -> bool {
    occupant_depuis_ns != 0 && maintenant_ns.saturating_sub(occupant_depuis_ns) > seuil_ns
}

/// Une tache prete depuis `pret_depuis_ns`, en file derriere un occupant
/// installe depuis `occupant_depuis_ns`, peut-elle etre relogee ?
///
/// Les deux attentes doivent passer le seuil : un occupant ancien qu'une
/// tache tout juste prete va faire ceder n'est pas un coeur muet pour elle.
pub fn secours_permis(occupant_depuis_ns: u64, pret_depuis_ns: u64, maintenant_ns: u64, seuil_ns: u64) -> bool {
    coeur_muet(occupant_depuis_ns, maintenant_ns, seuil_ns)
        && pret_depuis_ns != 0
        && maintenant_ns.saturating_sub(pret_depuis_ns) > seuil_ns
}

/// Un coeur candidat a l'accueil, tel que l'appelant l'a lu.
#[derive(Clone, Copy, Debug, Default)]
pub struct Accueil {
    /// L'affinite de la tache le permet.
    pub permis: bool,
    /// Le coeur dort, sans tache.
    pub inactif: bool,
    /// Le coeur est muet lui aussi.
    pub muet: bool,
    /// Taches en attente dans sa file.
    pub attente: usize,
}

/// A sleeping task can remain installed across HLT for seconds. Its old
/// installation timestamp is not evidence that the CPU stopped scheduling.
/// Conversely the BSP shell is not an idle scheduler, even while it waits.
pub fn accueil(permis: bool, inactif: bool, hors_ordonnanceur: bool,
    occupant_depuis: u64, maintenant: u64, seuil: u64, attente: usize) -> Accueil {
    Accueil {
        permis: permis && !hors_ordonnanceur,
        inactif,
        muet: !inactif && coeur_muet(occupant_depuis, maintenant, seuil),
        attente,
    }
}

/// Le coeur d'accueil : jamais le donneur, jamais un coeur interdit ou muet ;
/// un coeur au repos d'abord, sinon la file la plus courte (le plus petit
/// indice a egalite).
pub fn choisit_accueil(donneur: usize, coeurs: &[Accueil]) -> Option<usize> {
    let mut meilleur: Option<(usize, (bool, usize))> = None;
    for (cpu, c) in coeurs.iter().enumerate() {
        if cpu == donneur || !c.permis || c.muet {
            continue;
        }
        // Rang : inactif d'abord (`false` < `true`), puis l'attente.
        let rang = (!c.inactif, c.attente);
        if meilleur.map_or(true, |(_, r)| rang < r) {
            meilleur = Some((cpu, rang));
        }
    }
    meilleur.map(|(cpu, _)| cpu)
}

/// Les coeurs dont une tache en file peut etre secourue : file non vide et
/// occupant installe depuis plus de `seuil_ns` ([`coeur_muet`]). Un masque
/// (bit `cpu`) ; vide dans le cas courant, qui ne coute alors qu'une lecture
/// par coeur. `coeurs[cpu] = (occupant_depuis_ns, file_non_vide)`.
pub fn coeurs_muets(coeurs: &[(u64, bool)], maintenant_ns: u64, seuil_ns: u64) -> u64 {
    let mut masque = 0u64;
    for (cpu, &(depuis, file)) in coeurs.iter().enumerate().take(64) {
        if file && coeur_muet(depuis, maintenant_ns, seuil_ns) {
            masque |= 1 << cpu;
        }
    }
    masque
}
