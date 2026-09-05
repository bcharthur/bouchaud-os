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

// ---------------------------------------------------------------------------
// Le pave de navigation et les touches de fonction
//
// Ce que ces tests gardent : des touches qui ETAIENT PERDUES. Le decodeur ne
// reconnaissait, parmi les sequences etendues, que les quatre fleches ; il
// rendait `None` pour tout le reste. Origine, Fin, Page precedente et Page
// suivante n'atteignaient donc jamais un client -- sans consequence visible sur
// le bureau, mais un navigateur ou l'on ne peut pas faire defiler une page sans
// molette n'est pas un navigateur.
//
// Une touche perdue ne fait echouer aucun test de boot : elle ne produit rien,
// et rien n'est exactement ce qu'on attend d'un octet inconnu.
// ---------------------------------------------------------------------------

const SC_ORIGINE: u8 = 0x47;
const SC_FIN: u8 = 0x4f;
const SC_PAGE_HAUT: u8 = 0x49;
const SC_PAGE_BAS: u8 = 0x51;
const SC_INSER: u8 = 0x52;
const SC_SUPPR: u8 = 0x53;

#[test]
fn le_pave_de_navigation_produit_ses_touches() {
    let attendus = [
        (SC_ORIGINE, Key::Home),
        (SC_FIN, Key::End),
        (SC_PAGE_HAUT, Key::PageUp),
        (SC_PAGE_BAS, Key::PageDown),
        (SC_INSER, Key::Insert),
        (SC_SUPPR, Key::Delete),
    ];

    for (scancode, touche) in attendus {
        let evenements = joue(&[0xe0, scancode, 0xe0, scancode | RELACHE]);
        assert_eq!(
            evenements.len(),
            2,
            "0xE0 {:#04x} doit produire un appui et un relachement",
            scancode
        );
        assert_eq!(evenements[0].logique, touche);
        assert!(evenements[0].appui && evenements[0].etendue);
        assert_eq!(evenements[1].logique, touche);
        assert!(!evenements[1].appui);
    }
}

#[test]
fn suppr_n_est_pas_retour_arriere() {
    // Le defaut precis, et le plus vicieux des trois : `0xE0 0x53` etait traduit
    // en `Key::Backspace`. La touche existait, arrivait, et faisait l'inverse de
    // ce qu'elle annonce -- elle effacait le caractere de GAUCHE. Rien ne
    // distinguait cela d'un utilisateur qui se trompe de touche.
    let evenements = joue(&[0xe0, SC_SUPPR]);
    assert_eq!(evenements.len(), 1);
    assert_eq!(evenements[0].logique, Key::Delete);
    assert_ne!(evenements[0].logique, Key::Backspace);

    // La reciproque : Retour arriere, lui, reste Retour arriere.
    let arriere = joue(&[0x0e]);
    assert_eq!(arriere.len(), 1);
    assert_eq!(arriere[0].logique, Key::Backspace);
}

#[test]
fn le_pave_numerique_ne_se_prend_pas_pour_le_pave_de_navigation() {
    // Les MEMES scancodes, sans le prefixe 0xE0, appartiennent au pave
    // numerique, dont la signification depend de Verr.Num -- un etat que ce
    // decodeur ne suit pas. Les accepter ferait taper « Origine » a qui appuie
    // sur 7, une fois sur deux, et le defaut serait intermittent.
    for scancode in [SC_ORIGINE, SC_FIN, SC_PAGE_HAUT, SC_PAGE_BAS, SC_INSER, SC_SUPPR] {
        let evenements = joue(&[scancode]);
        assert!(
            evenements.is_empty(),
            "{:#04x} nu appartient au pave numerique, pas au pave de navigation",
            scancode
        );
    }
}

#[test]
fn les_touches_de_fonction_portent_leur_numero() {
    // F1..F10 se suivent, puis F11 et F12 ont ete ajoutees a la fin du jeu 1.
    // C'est de l'histoire du materiel : un test vaut mieux qu'un souvenir.
    let cas = [(0x3b, 1u8), (0x3c, 2), (0x44, 10), (0x57, 11), (0x58, 12)];
    for (scancode, numero) in cas {
        let evenements = joue(&[scancode, scancode | RELACHE]);
        assert_eq!(evenements.len(), 2, "F{} doit sortir des deux cotes", numero);
        assert_eq!(evenements[0].logique, Key::Fonction(numero));
        // Le numero voyage aussi dans `unicode` : c'est ce que le protocole GUI
        // transporte, faute d'un code de touche par touche de fonction.
        assert_eq!(evenements[0].unicode, numero as u32);
        assert_eq!(evenements[1].logique, Key::Fonction(numero));
    }
}

#[test]
fn f5_n_est_pas_un_caractere() {
    // Le raccourci de rechargement du navigateur. S'il arrivait comme un
    // caractere, il s'inserirait dans la barre d'adresse au lieu de recharger.
    let evenements = joue(&[0x3f]);
    assert_eq!(evenements.len(), 1);
    assert_eq!(evenements[0].logique, Key::Fonction(5));
    assert!(!matches!(evenements[0].logique, Key::Char(_)));
}

#[test]
fn entree_du_pave_numerique_vaut_entree() {
    // `0xE0 0x1C`. Sans elle, valider une URL depuis le pave numerique ne
    // faisait rien du tout.
    let evenements = joue(&[0xe0, 0x1c]);
    assert_eq!(evenements.len(), 1);
    assert_eq!(evenements[0].logique, Key::Enter);
}

#[test]
fn une_sequence_etendue_inconnue_reste_sans_effet() {
    // La reciproque des tests ci-dessus : elargir la table ne doit pas revenir
    // a tout accepter. Les touches multimedia (0xE0 0x22 « lecture/pause », par
    // exemple) n'ont aucune signification pour ce systeme, et en inventer une
    // produirait des frappes que personne n'a faites.
    assert!(joue(&[0xe0, 0x22]).is_empty());
    assert!(joue(&[0xe0, 0x6d]).is_empty());
}
