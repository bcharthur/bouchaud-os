//! Harnais de test hote pour le decodage clavier.
//!
//! Meme principe que `test_protocole.rs` : le noyau se compile pour
//! `x86_64-bouchaud_os`, sans `std` ni harnais de test, alors que le decodage
//! d'un scancode est une fonction pure. `src/drivers/input/clavier_decodeur.rs`
//! est inclus tel quel -- le code exerce est exactement celui qui tourne sur la
//! machine.
//!
//! Ce qu'un decodeur clavier casse en silence : une touche perdue passe pour
//! une frappe ratee, un relachement perdu pour une bizarrerie de la page. Rien
//! dans un boot ne le signale.
//!
//! Lance par `tools/gui/test-clavier.sh`.

#[path = "../../src/drivers/input/clavier_decodeur.rs"]
mod decodeur;

use decodeur::{EtatClavier, Key, KeyEvent};

/// Deroule une suite de scancodes et rend les transitions produites.
fn joue(codes: &[u8]) -> Vec<KeyEvent> {
    let mut etat = EtatClavier::neuf();
    codes.iter().filter_map(|&sc| etat.decode(sc)).collect()
}

/// Scancode AZERTY des lettres utilisees ici (jeu 1).
const SC_A: u8 = 0x10; // 'a' en AZERTY (position Q d'un QWERTY)
const SC_B: u8 = 0x30;
const SC_C: u8 = 0x2e;
const RELACHE: u8 = 0x80;

const SC_SHIFT_G: u8 = 0x2a;
const SC_CTRL: u8 = 0x1d;

#[test]
fn taper_abc_produit_trois_appuis_et_trois_relachements() {
    // C'est le scenario exact du critere d'acceptation : trois lettres tapees,
    // trois appuis et trois relachements, dans l'ordre, et rien d'autre.
    let evenements = joue(&[
        SC_A, SC_A | RELACHE,
        SC_B, SC_B | RELACHE,
        SC_C, SC_C | RELACHE,
    ]);

    assert_eq!(evenements.len(), 6, "trois frappes = six transitions");

    let appuis: Vec<_> = evenements.iter().filter(|e| e.appui).collect();
    let relachements: Vec<_> = evenements.iter().filter(|e| !e.appui).collect();
    assert_eq!(appuis.len(), 3, "trois appuis");
    assert_eq!(relachements.len(), 3, "trois relachements");

    // L'alternance compte autant que le compte : six appuis suivis de six
    // relachements passeraient les deux assertions precedentes.
    for (rang, evenement) in evenements.iter().enumerate() {
        assert_eq!(evenement.appui, rang % 2 == 0, "transition {}", rang);
    }

    assert_eq!(appuis[0].logique, Key::Char(b'a'));
    assert_eq!(appuis[1].logique, Key::Char(b'b'));
    assert_eq!(appuis[2].logique, Key::Char(b'c'));

    // Aucune repetition : chaque appui a ete precede d'un relachement.
    assert!(evenements.iter().all(|e| !e.repeat), "aucune repetition attendue");
}

#[test]
fn un_relachement_porte_la_meme_touche_que_son_appui() {
    // Le client se sert de la touche pour apparier `keydown` et `keyup` : si
    // le relachement portait une autre touche, une page qui suit l'etat du
    // clavier resterait persuadee que la premiere est toujours enfoncee.
    let evenements = joue(&[SC_A, SC_A | RELACHE]);
    assert_eq!(evenements.len(), 2);
    assert_eq!(evenements[0].logique, evenements[1].logique);
    assert_eq!(evenements[0].scancode, evenements[1].scancode);
    assert!(evenements[0].appui && !evenements[1].appui);
}

#[test]
fn une_touche_maintenue_se_declare_repetition() {
    // Le controleur renvoie le meme code d'appui sans relachement. Sans cette
    // distinction, une touche laissee enfoncee serait indiscernable d'une
    // rafale de frappes distinctes.
    let evenements = joue(&[SC_A, SC_A, SC_A, SC_A | RELACHE]);
    assert_eq!(evenements.len(), 4);
    assert!(!evenements[0].repeat, "la premiere frappe n'est pas une repetition");
    assert!(evenements[1].repeat);
    assert!(evenements[2].repeat);
    assert!(!evenements[3].repeat, "un relachement n'est jamais une repetition");
}

#[test]
fn un_modificateur_ne_produit_aucune_touche_mais_change_les_suivantes() {
    let mut etat = EtatClavier::neuf();

    // Shift seul : aucun evenement, mais l'etat bascule.
    assert!(etat.decode(SC_SHIFT_G).is_none(), "Shift ne produit pas de touche");
    assert!(etat.modificateurs().shift);

    let majuscule = etat.decode(SC_A).expect("la lettre sort");
    assert_eq!(majuscule.logique, Key::Char(b'A'));
    assert!(majuscule.modificateurs.shift, "le masque accompagne la touche");
    let _ = etat.decode(SC_A | RELACHE);

    // Et le relachement de Shift est pris : sans lui, tout resterait en
    // majuscules pour toujours.
    assert!(etat.decode(SC_SHIFT_G | RELACHE).is_none());
    assert!(!etat.modificateurs().shift);

    let minuscule = etat.decode(SC_A).expect("la lettre sort");
    assert_eq!(minuscule.logique, Key::Char(b'a'));
    assert!(!minuscule.modificateurs.shift);
}

#[test]
fn ctrl_accompagne_la_touche_sans_la_remplacer() {
    // Ctrl+A doit rester la touche 'a' AVEC le modificateur : une page teste
    // `event.ctrlKey && event.key == "a"`, pas un code special.
    let mut etat = EtatClavier::neuf();
    assert!(etat.decode(SC_CTRL).is_none());
    let evenement = etat.decode(SC_A).expect("la lettre sort");
    assert_eq!(evenement.logique, Key::Char(b'a'));
    assert!(evenement.modificateurs.ctrl);
    assert!(!evenement.modificateurs.shift);

    assert!(etat.decode(SC_CTRL | RELACHE).is_none());
    assert!(!etat.modificateurs().ctrl);
}

#[test]
fn les_touches_etendues_sortent_des_deux_cotes() {
    // Prefixe 0xE0 puis code : les fleches. Elles avaient des appuis mais
    // aucun relachement dans l'ancien decodeur.
    const HAUT: u8 = 0x48;
    let evenements = joue(&[0xe0, HAUT, 0xe0, HAUT | RELACHE]);
    assert_eq!(evenements.len(), 2);
    assert_eq!(evenements[0].logique, Key::Up);
    assert!(evenements[0].appui && evenements[0].etendue);
    assert_eq!(evenements[1].logique, Key::Up);
    assert!(!evenements[1].appui && evenements[1].etendue);
}

#[test]
fn altgr_ne_se_confond_pas_avec_alt_gauche() {
    // Meme scancode 0x38 : seul le prefixe 0xE0 les distingue. Les confondre
    // ferait produire des accolades a chaque Alt gauche.
    let mut etat = EtatClavier::neuf();
    assert!(etat.decode(0x38).is_none());
    assert!(etat.modificateurs().alt, "0x38 nu = Alt gauche");
    assert!(!etat.modificateurs().altgr);
    assert!(etat.decode(0x38 | RELACHE).is_none());

    assert!(etat.decode(0xe0).is_none());
    assert!(etat.decode(0x38).is_none());
    assert!(etat.modificateurs().altgr, "0xE0 0x38 = AltGr");
    assert!(!etat.modificateurs().alt);
}

#[test]
fn la_touche_logique_seule_ignore_les_relachements() {
    // `decode_touche` sert le shell et l'invite de connexion : eux veulent un
    // caractere, pas une transition. Une lettre tapee doit y compter une fois.
    let mut etat = EtatClavier::neuf();
    let mut vues = Vec::new();
    for sc in [SC_A, SC_A | RELACHE, SC_B, SC_B | RELACHE] {
        if let Some(k) = etat.decode_touche(sc) {
            vues.push(k);
        }
    }
    assert_eq!(vues, vec![Key::Char(b'a'), Key::Char(b'b')]);
}
