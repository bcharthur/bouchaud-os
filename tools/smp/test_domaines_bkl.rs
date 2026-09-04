//! L'attribution des prises du gros verrou peut-elle mentir ?
//!
//! # Ce qui est en jeu
//!
//! La sortie du gros verrou se fait chemin par chemin, et un chemin sorti doit
//! ensuite le RESTER. Le seul instrument qui permette d'affirmer l'un et
//! l'autre est l'attribution : combien d'acquisitions viennent de quel
//! sous-systeme, et laquelle vient d'un sous-systeme qui avait promis de ne
//! plus en prendre.
//!
//! Un instrument faux est pire qu'aucun instrument. Deux facons de se tromper
//! comptent ici :
//!
//!   * attribuer au mauvais domaine -- on instrumente alors le mauvais code ;
//!   * ne pas attribuer du tout -- le total des chemins normaux devient un
//!     minorant, et « ca baisse » ne veut plus rien dire.
//!
//! Lance par `tools/dev/validate-fast.ps1`.

#[path = "../../src/kernel/sync/domaine.rs"]
mod domaine;

use domaine::{Contrat, Domaine, Registre, MAX_CPUS, NOMBRE, PROFONDEUR};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// 1. Qui paie : le domaine le plus INTERIEUR
// ---------------------------------------------------------------------------

#[test]
fn c_est_le_domaine_le_plus_interieur_qui_paie() {
    // Un appel systeme entre dans `Fd`, qui ecrit sur un peripherique. C'est
    // `Pilote` qui a eu besoin du verrou, et c'est lui qu'il faudra
    // instrumenter -- pas `Fd`, qui n'a fait que passer.
    //
    // L'exemple etait `Fs` tant que le systeme de fichiers prenait encore le
    // gros verrou. Il a son propre verrou depuis, donc son contrat est `Migre`
    // et `note_acquisition` rendrait `Some(Fs)` : ce test-ci mesure QUI PAIE,
    // pas qui a promis, et il lui faut un domaine encore Legacy pour ne pas
    // confondre les deux proprietes.
    let r = Registre::neuf();
    r.entre(0, Domaine::Syscall);
    r.entre(0, Domaine::Fd);
    r.entre(0, Domaine::Pilote);

    assert_eq!(r.courant(0), Domaine::Pilote);
    assert_eq!(r.note_acquisition(0), None, "Pilote n'est pas declare sorti");
    assert_eq!(r.acquisitions(Domaine::Pilote), 1);
    assert_eq!(r.acquisitions(Domaine::Fd), 0);
    assert_eq!(r.acquisitions(Domaine::Syscall), 0);

    // En ressortant, l'attribution revient au domaine englobant.
    r.sort(0);
    assert_eq!(r.courant(0), Domaine::Fd);
    r.note_acquisition(0);
    assert_eq!(r.acquisitions(Domaine::Fd), 1);
}

#[test]
fn hors_de_toute_portee_l_acquisition_est_indeterminee() {
    // Ce cas doit rester VISIBLE : c'est du code non encore rattache, et le
    // garde-fou source existe pour qu'il n'y en ait aucun.
    let r = Registre::neuf();
    assert_eq!(r.courant(0), Domaine::Indetermine);
    r.note_acquisition(0);
    assert_eq!(r.acquisitions(Domaine::Indetermine), 1);
    assert_eq!(
        r.acquisitions_chemins_normaux(), 0,
        "une acquisition non rattachee ne doit pas gonfler le chiffre des \
         chemins normaux : elle le rendrait faussement mauvais",
    );
}

// ---------------------------------------------------------------------------
// 2. Le contrat : un chemin sorti qui reprend le verrou est une regression
// ---------------------------------------------------------------------------

#[test]
fn un_domaine_migre_qui_reprend_le_verrou_est_signale() {
    let r = Registre::neuf();
    r.entre(0, Domaine::VerrouEnregistrement);
    assert_eq!(
        r.note_acquisition(0), Some(Domaine::VerrouEnregistrement),
        "le contrat de ce domaine est `Migre` : la reprise doit etre rendue a \
         l'appelant, pas seulement comptee",
    );
    assert_eq!(r.violations(Domaine::VerrouEnregistrement), 1);
    assert_eq!(r.total_violations(), 1);
}

#[test]
fn on_garde_la_premiere_regression_pas_la_derniere() {
    // La premiere est celle qui a introduit le defaut ; les suivantes peuvent
    // n'en etre que la consequence.
    let r = Registre::neuf();
    r.entre(0, Domaine::VerrouEnregistrement);
    r.note_acquisition(0);
    r.sort(0);
    r.entre(0, Domaine::RegistreProcessus);
    r.note_acquisition(0);
    r.note_acquisition(0);

    assert_eq!(r.premiere_regression(), Some(Domaine::VerrouEnregistrement));
    assert_eq!(r.violations(Domaine::RegistreProcessus), 2);
    assert_eq!(r.total_violations(), 3);
}

#[test]
fn un_domaine_non_migre_compte_sans_accuser() {
    let r = Registre::neuf();
    for domaine in [Domaine::Vm, Domaine::Fd, Domaine::Reseau, Domaine::Pilote] {
        r.entre(0, domaine);
        assert_eq!(r.note_acquisition(0), None, "{:?} n'a rien promis", domaine);
        r.sort(0);
    }
    assert_eq!(r.total_violations(), 0);
    assert_eq!(r.acquisitions_chemins_normaux(), 4);
}

#[test]
fn le_boot_et_la_panique_ne_comptent_pas_comme_chemins_normaux() {
    // Le gros verrou y reste legitime pour toujours : il n'y a pas de
    // concurrence a proteger au boot, et une panique n'a pas a etre elegante.
    // Les inclure rendrait l'objectif « zero » inatteignable par construction.
    let r = Registre::neuf();
    r.entre(0, Domaine::BootPrecoce);
    r.note_acquisition(0);
    r.sort(0);
    r.entre(0, Domaine::Panique);
    r.note_acquisition(0);
    r.sort(0);

    assert_eq!(r.total_violations(), 0);
    assert_eq!(r.acquisitions_chemins_normaux(), 0);
    assert_eq!(r.acquisitions(Domaine::BootPrecoce), 1);
}

#[test]
fn tout_domaine_a_un_contrat_et_les_migres_sont_ceux_qu_on_croit() {
    // Un domaine ajoute sans contrat relu tomberait dans un defaut de `match`
    // et se retrouverait silencieusement `Legacy`.
    let mut migres = Vec::new();
    for code in 0..NOMBRE as u8 {
        let d = Domaine::depuis_code(code);
        assert!(!d.nom().is_empty());
        assert!(!d.contrat().nom().is_empty());
        if matches!(d.contrat(), Contrat::Migre) {
            migres.push(d);
        }
    }
    assert_eq!(
        migres,
        vec![
            Domaine::Ordonnanceur,
            // La readiness a rejoint la liste au chantier 1 : sa DERNIERE
            // reprise vivait sur la branche legacy de `WaitQueue::wait`, la ou
            // l'appelant tenait deja le verrou. Reprendre un verrou reentrant
            // que ce CPU detient deja n'ajoute aucune exclusion ; la reprise a
            // ete supprimee, et `[MM-NG6] waitq_bkl_wait_ns=0` en est la
            // mesure.
            Domaine::Readiness,
            Domaine::Vfs,
            Domaine::Fs,
            Domaine::RegistreProcessus,
            Domaine::VerrouEnregistrement,
        ],
        "la liste des domaines sortis est le CONTRAT du chantier : elle ne \
         change qu'avec une migration reelle",
    );
}

#[test]
fn le_code_et_le_domaine_sont_bijectifs() {
    // L'attribution passe par un `u8` dans un atomique. Un aller-retour qui ne
    // serait pas l'identite attribuerait au mauvais sous-systeme.
    for code in 0..NOMBRE as u8 {
        assert_eq!(Domaine::depuis_code(code).code(), code);
    }
}

// ---------------------------------------------------------------------------
// 3. La pile : imbrication, debordement, isolation par CPU
// ---------------------------------------------------------------------------

#[test]
fn les_piles_des_cpu_sont_independantes() {
    // Deux coeurs dans deux sous-systemes differents au meme instant : c'est le
    // cas NOMINAL en SMP, pas un cas limite.
    let r = Registre::neuf();
    r.entre(0, Domaine::Reseau);
    r.entre(1, Domaine::Vfs);

    assert_eq!(r.courant(0), Domaine::Reseau);
    assert_eq!(r.courant(1), Domaine::Vfs);
    r.note_acquisition(0);
    r.note_acquisition(1);
    assert_eq!(r.acquisitions(Domaine::Reseau), 1);
    assert_eq!(r.acquisitions(Domaine::Vfs), 1);
}

#[test]
fn un_debordement_se_compte_et_ne_ment_pas() {
    // Au-dela de la profondeur suivie on garde le plus profond CONNU. Rendre
    // `Indetermine` serait plus simple et bien pire : on perdrait l'attribution
    // exactement quand le chemin est le plus complique.
    let r = Registre::neuf();
    for _ in 0..PROFONDEUR {
        r.entre(0, Domaine::Vm);
    }
    let profond = r.courant(0);
    for _ in 0..4 {
        r.entre(0, Domaine::Fs);
    }
    assert_eq!(r.debordements(), 4);
    assert_eq!(r.courant(0), profond, "on rend le plus profond connu");

    // Et la pile se referme exactement : autant de `sort` que de `entre`.
    for _ in 0..(PROFONDEUR + 4) {
        r.sort(0);
    }
    assert_eq!(r.profondeur(0), 0);
    assert_eq!(r.courant(0), Domaine::Indetermine);
}

#[test]
fn la_pile_ne_passe_jamais_sous_zero() {
    // Un `sort` sans `entre` est impossible avec la RAII, mais une soustraction
    // qui deborderait ferait de la profondeur un nombre enorme, et toute
    // lecture ulterieure lirait hors du tableau.
    let r = Registre::neuf();
    r.sort(0);
    r.sort(0);
    assert_eq!(r.profondeur(0), 0);
    assert_eq!(r.courant(0), Domaine::Indetermine);
}

#[test]
fn un_index_de_cpu_hors_bornes_ne_deborde_aucun_tableau() {
    // La colle noyau borne deja l'index. Le module ne doit pas en dependre :
    // il s'execute sur le chemin d'acquisition du verrou, interruptions
    // masquees, ou une panic serait fatale.
    let r = Registre::neuf();
    r.entre(MAX_CPUS + 3, Domaine::Fs);
    r.sort(MAX_CPUS + 3);
    assert_eq!(r.courant(MAX_CPUS + 3), Domaine::Indetermine);
    assert_eq!(r.profondeur(usize::MAX), 0);
    r.note_acquisition(MAX_CPUS + 3);
}

// ---------------------------------------------------------------------------
// 4. Sous concurrence reelle
// ---------------------------------------------------------------------------

#[test]
fn le_compte_est_exact_sous_concurrence() {
    let r = Arc::new(Registre::neuf());
    let tours = 20_000u64;
    let coeurs = 4usize;

    let mut fils = Vec::new();
    for cpu in 0..coeurs {
        let r = Arc::clone(&r);
        fils.push(std::thread::spawn(move || {
            for _ in 0..tours {
                r.entre(cpu, Domaine::Syscall);
                r.entre(cpu, Domaine::Pilote);
                r.note_acquisition(cpu);
                r.sort(cpu);
                r.sort(cpu);
            }
        }));
    }
    for f in fils {
        f.join().unwrap();
    }

    assert_eq!(
        r.acquisitions(Domaine::Pilote), tours * coeurs as u64,
        "aucune acquisition ne doit se perdre : le chiffre sert de critere",
    );
    assert_eq!(r.acquisitions(Domaine::Syscall), 0);
    assert_eq!(r.total_violations(), 0);
    assert_eq!(r.debordements(), 0);
    for cpu in 0..coeurs {
        assert_eq!(r.profondeur(cpu), 0, "toutes les portees sont refermees");
    }
}
