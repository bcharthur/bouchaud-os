//! Ce que `madvise` et `poll` ont le droit de faire sous le gros verrou.
//!
//! # Ce que ces tests verifient
//!
//! Les traces ci-dessous decrivent les chemins REELS a leur etat actuel : les
//! prises, les suspensions, les reprises et les phases de travail, dans
//! l'ordre ou le code les execute. Chaque phase declare de quoi son cout
//! depend, et c'est cette declaration que le verificateur juge.
//!
//! La regle centrale :
//!
//!     Sous le gros verrou, le cout d'une phase peut dependre de la taille de
//!     la DEMANDE. Jamais de la taille d'un ETAT GLOBAL.
//!
//! Les trois defauts trouves au runtime violaient tous celle-la, et chacun est
//! rejoue ici comme trace fautive : la verification doit les refuser.
//!
//! # Ce que ces tests ne prouvent pas
//!
//! Que la trace decrit fidelement le code. C'est une lecture, pas une mesure.
//! Ce qu'ils prouvent, c'est que si quelqu'un ajoute une phase globale sous le
//! verrou, il devra l'ecrire dans la table -- et le test la refusera.
//!
//! Lance par `tools/smp/test-discipline-bkl.sh`.

extern crate alloc;

#[path = "../../src/kernel/sync/discipline.rs"]
mod discipline;

use discipline::{verifie, Cout, Evenement, Faute};
use Evenement::{Dort, Phase, Prend, Rend, Reprend, Suspend};

// ─── Les chemins reels ─────────────────────────────────────────────────────

/// `sys_madvise(MADV_DONTNEED)`, tel qu'il est apres correction.
///
/// `usermode.rs` prend le verrou pour tout l'appel (`madvise` n'est pas dans
/// `SANS_BKL`), puis :
///   validation sous `mm.lock()`, preparation de la plage, shootdown TLB avec
///   le verrou SUSPENDU, finition, retour des pages propres.
fn madvise_actuel() -> alloc::vec::Vec<Evenement> {
    alloc::vec![
        Prend,                                              // usermode.rs
        Phase("vm_validation", Cout::LineaireEnDemande),    // vma::couvre
        Phase("retire_clean_pages", Cout::LineaireEnDemande),
        Phase("prepare_unmap", Cout::Logarithmique),        // BTreeSet::contains
        Suspend,                                            // execute_process_invalidation
        Dort("attente ACK TLB des CPU distants"),
        Reprend,
        Phase("finish_unmap", Cout::Logarithmique),         // BTreeSet::remove
        Phase("free_frame", Cout::Constant),                // bitmap, un bit
        Phase("clean_page_cache::release", Cout::Logarithmique), // BTreeMap
        Rend,
    ]
}

/// `sys_poll`, tel qu'il est depuis qu'il est libere du gros verrou.
///
/// `usermode.rs` ne prend plus rien : `POLL` figure dans `SANS_BKL`. Le
/// balayage n'utilise que le verrou de la table des descripteurs et celui de
/// chaque objet. Trois branches touchent un etat global sans verrou propre --
/// clavier, souris, socket inet -- et prennent le gros verrou elles-memes, au
/// plus court. L'attente le prend aussi, le temps de s'inscrire, puis le
/// suspend pour de bon avant de commuter.
fn poll_actuel() -> alloc::vec::Vec<Evenement> {
    alloc::vec![
        // Aucun `Prend` d'entree : l'appel est libere.
        Phase("readiness_ticket", Cout::Constant),
        Phase("balayage des descripteurs", Cout::LineaireEnDemande),
        // Une branche a etat global, prise au plus court.
        Prend,
        Phase("socket_readable (anneau e1000)", Cout::Constant),
        Rend,
        // L'attente : inscription sous le verrou, puis suspension.
        Prend,
        Phase("inscription sur la WaitQueue", Cout::Constant),
        Suspend,
        Dort("park_current_on"),
        Reprend,
        Rend,
        Phase("balayage des descripteurs", Cout::LineaireEnDemande),
    ]
}

/// Ce que `poll` faisait AVANT : le verrou pris pour tout l'appel.
///
/// La trace reste correcte au regard de la discipline -- rien n'y dort sous le
/// verrou, aucune phase n'y est globale. C'est justement ce que ce test dit :
/// la discipline ne suffisait pas a trouver ce probleme-la, et c'est la mesure
/// par appel systeme (`[BKL-SYSCALL]`) qui l'a nomme.
fn poll_avant_liberation() -> alloc::vec::Vec<Evenement> {
    alloc::vec![
        Prend,                                              // usermode.rs
        Phase("readiness_ticket", Cout::Constant),
        Phase("balayage des descripteurs", Cout::LineaireEnDemande),
        Suspend,                                            // schedule()
        Dort("park_current_on"),
        Reprend,
        Phase("balayage des descripteurs", Cout::LineaireEnDemande),
        Rend,
    ]
}

#[test]
fn madvise_respecte_la_discipline() {
    assert_eq!(verifie(&madvise_actuel()), Ok(()));
}

#[test]
fn poll_respecte_la_discipline() {
    assert_eq!(verifie(&poll_actuel()), Ok(()));
}

/// L'ancienne forme la respectait AUSSI. Ce n'est pas un aveu d'echec de la
/// regle : c'est sa portee. Tenir le verrou tres longtemps sans jamais dormir
/// ni rien faire de global est parfaitement discipline, et parfaitement
/// desastreux pour trois autres coeurs. C'est `[BKL-SYSCALL]` qui l'a montre.
#[test]
fn l_ancienne_forme_de_poll_respectait_deja_la_discipline() {
    assert_eq!(
        verifie(&poll_avant_liberation()),
        Ok(()),
        "la discipline ne dit rien de la DUREE d'une tenue, seulement de ce \
         qu'on a le droit d'y faire"
    );
}

/// Ce que la liberation change, et qui se lit dans la trace : le verrou n'est
/// plus tenu pendant le balayage.
#[test]
fn le_balayage_de_poll_ne_tient_plus_le_verrou() {
    let mut profondeur = 0i32;
    let mut balayages_sous_verrou = 0;
    for evenement in poll_actuel() {
        match evenement {
            Prend => profondeur += 1,
            Rend => profondeur -= 1,
            Suspend => profondeur = 0,
            Phase("balayage des descripteurs", _) if profondeur > 0 => {
                balayages_sous_verrou += 1;
            }
            _ => {}
        }
    }
    assert_eq!(
        balayages_sous_verrou, 0,
        "le balayage des descripteurs ne doit plus s'executer verrou tenu"
    );

    // Et l'ancienne forme, elle, le faisait : c'est la difference mesuree.
    let mut profondeur = 0i32;
    let mut avant = 0;
    for evenement in poll_avant_liberation() {
        match evenement {
            Prend => profondeur += 1,
            Rend => profondeur -= 1,
            Suspend => profondeur = 0,
            Phase("balayage des descripteurs", _) if profondeur > 0 => avant += 1,
            _ => {}
        }
    }
    assert!(avant > 0, "l'ancienne forme balayait bien sous le verrou");
}

// ─── Les trois defauts reels, rejoues ──────────────────────────────────────

/// `free_frame` parcourait la liste libre entiere a chaque liberation.
#[test]
fn madvise_qui_balaye_la_liste_libre_est_refuse() {
    let mut trace = madvise_actuel();
    trace[8] = Phase("free_frame", Cout::LineaireEnEtatGlobal);
    assert_eq!(
        verifie(&trace),
        Err(Faute::PhaseGlobaleSousVerrou { phase: "free_frame", profondeur: 1 }),
    );
}

/// `prepare_unmap` / `finish_unmap` balayaient toutes les frames residentes.
#[test]
fn madvise_qui_balaye_les_frames_residentes_est_refuse() {
    let mut trace = madvise_actuel();
    trace[3] = Phase("prepare_unmap", Cout::LineaireEnEtatGlobal);
    assert!(matches!(
        verifie(&trace),
        Err(Faute::PhaseGlobaleSousVerrou { phase: "prepare_unmap", .. })
    ));

    let mut trace = madvise_actuel();
    trace[7] = Phase("finish_unmap", Cout::LineaireEnEtatGlobal);
    assert!(matches!(
        verifie(&trace),
        Err(Faute::PhaseGlobaleSousVerrou { phase: "finish_unmap", .. })
    ));
}

/// `release` recomptait tout le cache de pages propres, a chaque page rendue.
#[test]
fn madvise_qui_recompte_le_cache_de_pages_est_refuse() {
    let mut trace = madvise_actuel();
    trace[9] = Phase("clean_page_cache::release", Cout::LineaireEnEtatGlobal);
    assert!(matches!(
        verifie(&trace),
        Err(Faute::PhaseGlobaleSousVerrou { .. })
    ));
}

// ─── Dormir sous le verrou ─────────────────────────────────────────────────

/// LE defaut que le veilleur doit attraper : `poll` qui dort le verrou tenu.
#[test]
fn poll_qui_dort_sous_le_verrou_est_refuse() {
    // On retire la suspension -- par valeur, pas par position : la trace de
    // `poll` a change quand l'appel a ete libere du gros verrou, et un test qui
    // vise un index se serait tu au lieu d'echouer.
    let mut trace = poll_actuel();
    let position = trace
        .iter()
        .position(|evenement| *evenement == Suspend)
        .expect("la trace de poll doit suspendre avant de dormir");
    trace.remove(position);
    let attendu = Err(Faute::DortSousVerrou {
        quoi: "park_current_on",
        profondeur: 1,
    });
    assert_eq!(verifie(&trace), attendu);
}

/// Le shootdown TLB attend des CPU distants. S'il le faisait verrou tenu, les
/// CPU dont on attend l'acquittement pourraient etre en train de l'attendre.
#[test]
fn un_shootdown_sous_le_verrou_est_refuse() {
    let mut trace = madvise_actuel();
    trace.remove(4); // la suspension avant l'attente d'ACK
    assert!(matches!(verifie(&trace), Err(Faute::DortSousVerrou { .. })));
}

#[test]
fn dormir_sans_rien_tenir_est_permis() {
    assert_eq!(
        verifie(&[Dort("boucle idle"), Prend, Suspend, Dort("HLT"), Reprend, Rend]),
        Ok(())
    );
}

// ─── Bloquer apres la reprise sans rendre ──────────────────────────────────

/// Ce que le troisieme test demande : un chemin qui reprend le verrou apres un
/// changement de contexte, puis bloque de nouveau sans le rendre.
#[test]
fn bloquer_apres_reprise_sans_rendre_est_refuse() {
    let trace = alloc::vec![
        Prend,
        Suspend,
        Reprend,
        Phase("revalidation", Cout::LineaireEnDemande),
        Dort("seconde attente"),   // sans Suspend : faute
        Rend,
    ];
    assert_eq!(
        verifie(&trace),
        Err(Faute::DortSousVerrou { quoi: "seconde attente", profondeur: 1 }),
    );
}

#[test]
fn bloquer_apres_reprise_en_suspendant_est_permis() {
    let trace = alloc::vec![
        Prend, Suspend, Reprend,
        Phase("revalidation", Cout::LineaireEnDemande),
        Suspend, Dort("seconde attente"), Reprend,
        Rend,
    ];
    assert_eq!(verifie(&trace), Ok(()));
}

// ─── Comptabilite du verrou ────────────────────────────────────────────────

#[test]
fn la_reentrance_est_admise() {
    let trace = alloc::vec![
        Prend, Prend, Prend,
        Phase("travail", Cout::Constant),
        Rend, Rend, Rend,
    ];
    assert_eq!(verifie(&trace), Ok(()));
}

/// Une suspension rend TOUTE la profondeur, et la reprise la restaure.
#[test]
fn la_suspension_rend_toute_la_profondeur() {
    let trace = alloc::vec![
        Prend, Prend,
        Suspend,
        Dort("changement de contexte"),   // permis : profondeur 0
        Reprend,
        Rend, Rend,
    ];
    assert_eq!(verifie(&trace), Ok(()));
}

#[test]
fn une_reprise_qui_ne_restaure_pas_la_profondeur_se_voit() {
    // Reprendre a 1 alors qu'on tenait 2 : le second `Rend` part de zero.
    let trace = alloc::vec![Prend, Prend, Suspend, Reprend, Rend, Rend, Rend];
    assert_eq!(verifie(&trace), Err(Faute::RendSansPrendre));
}

#[test]
fn rendre_sans_prendre_est_refuse() {
    assert_eq!(verifie(&[Rend]), Err(Faute::RendSansPrendre));
}

#[test]
fn reprendre_sans_suspendre_est_refuse() {
    assert_eq!(verifie(&[Prend, Reprend, Rend]), Err(Faute::ReprendSansSuspendre));
}

#[test]
fn finir_en_tenant_le_verrou_est_refuse() {
    assert_eq!(
        verifie(&[Prend, Phase("travail", Cout::Constant)]),
        Err(Faute::FinitEnTenant { profondeur: 1 }),
    );
}

#[test]
fn suspendre_sans_reprendre_est_refuse() {
    assert_eq!(verifie(&[Prend, Suspend]), Err(Faute::SuspenduSansReprise));
}

/// Une phase globale HORS du verrou est parfaitement admise : c'est meme la
/// forme vers laquelle on veut pousser le travail long.
#[test]
fn une_phase_globale_hors_du_verrou_est_permise() {
    let trace = alloc::vec![
        Prend,
        Phase("instantane court", Cout::Logarithmique),
        Suspend,
        Phase("travail long", Cout::LineaireEnEtatGlobal),
        Reprend,
        Phase("commit court", Cout::Logarithmique),
        Rend,
    ];
    assert_eq!(verifie(&trace), Ok(()));
}

/// Un travail lineaire en la DEMANDE reste permis sous le verrou : mille pages
/// coutent mille fois une page, et c'est ce que l'appelant a demande.
#[test]
fn un_travail_lineaire_en_la_demande_reste_permis() {
    let trace = alloc::vec![
        Prend,
        Phase("plage de mille pages", Cout::LineaireEnDemande),
        Rend,
    ];
    assert_eq!(verifie(&trace), Ok(()));
}

#[test]
fn une_trace_vide_est_correcte() {
    assert_eq!(verifie(&[]), Ok(()));
}
