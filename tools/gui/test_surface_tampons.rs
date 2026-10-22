//! Preuve hote de la possession des tampons d'une surface client
//! (BOUCHAUD_GUI_SURFACE_DEUX_TAMPONS_V1, P10+.3c).
//!
//! `src/gui/surface_tampons.rs`, inclus tel quel. L'invariant central : le
//! compositeur ne lit jamais un tampon dans lequel un client CONFORME peut
//! ecrire -- c'est-a-dire qu'un tampon n'est jamais a la fois `Face` et
//! libre pour le client, et qu'aucune liberation ne se perd.

#[path = "../../src/gui/surface_tampons.rs"]
mod surface_tampons;

use surface_tampons::*;

/// Le modele d'un client conforme : il n'ecrit que dans un tampon qu'il sait
/// libre (jamais annonce, ou dont il a recu la liberation).
struct Client {
    libres: [bool; TAMPONS_MAX],
    nombre: usize,
}

impl Client {
    fn neuf(nombre: usize) -> Client {
        Client { libres: [true; TAMPONS_MAX], nombre }
    }
    fn choisit(&self) -> Option<u32> {
        (0..self.nombre).find(|&t| self.libres[t]).map(|t| t as u32)
    }
}

#[test]
fn v1_le_seul_tampon_est_toujours_la_face_et_rien_n_est_libere() {
    let mut t = Tampons::simple();
    for _ in 0..5 {
        assert_eq!(t.trame_prete(0), Issue::Acceptee);
        assert_eq!(t.face(), 0);
        assert_eq!(t.a_liberer(), None);
    }
    assert_eq!(t.trame_prete(1), Issue::HorsBornes);
    assert_eq!(t.face(), 0);
}

#[test]
fn la_negociation_exige_l_offre_et_la_capacite() {
    assert_eq!(Tampons::negocie(1, CAPACITE_DEUX_TAMPONS).nombre(), 1);
    assert_eq!(Tampons::negocie(2, 0).nombre(), 1);
    assert_eq!(Tampons::negocie(2, CAPACITE_DEUX_TAMPONS).nombre(), 2);
    assert_eq!(Tampons::negocie(2, CAPACITE_DEUX_TAMPONS | 0x80).nombre(), 2);
}

#[test]
fn la_capacite_se_lit_au_troisieme_mot_de_hello() {
    assert_eq!(capacites_hello(&[1, 0, 0, 0, 7, 0, 0, 0]), 0, "Hello V1 : huit octets");
    assert_eq!(capacites_hello(&[1, 0, 0, 0, 7, 0, 0, 0, 1, 0, 0, 0]), 1);
    assert_eq!(capacites_hello(&[]), 0);
}

#[test]
fn v2_changer_de_face_libere_l_ancienne_une_seule_fois() {
    let mut t = Tampons::double();
    assert_eq!(t.trame_prete(0), Issue::Acceptee);
    assert_eq!(t.a_liberer(), None, "premiere trame : aucune ancienne face");
    assert_eq!(t.trame_prete(1), Issue::Acceptee);
    assert_eq!(t.face(), 1);
    assert_eq!(t.a_liberer(), Some(0));
    assert!(t.liberation_envoyee(0));
    assert!(!t.liberation_envoyee(0), "une liberation ne part qu'une fois");
    assert_eq!(t.etat(0), Some(Etat::Libre));
    assert_eq!(t.a_liberer(), None);
}

#[test]
fn une_liberation_qui_n_a_pas_pu_partir_reste_due() {
    let mut t = Tampons::double();
    t.trame_prete(0);
    t.trame_prete(1);
    // Canal plein : l'envoi echoue, rien n'est marque.
    assert_eq!(t.a_liberer(), Some(0));
    assert_eq!(t.a_liberer(), Some(0), "toujours due au tour suivant");
    assert_eq!(t.etat(0), Some(Etat::ALiberer));
}

#[test]
fn reecrire_la_face_est_detecte_et_ne_libere_rien() {
    let mut t = Tampons::double();
    t.trame_prete(0);
    assert_eq!(t.trame_prete(0), Issue::EcritureSurFace);
    assert_eq!(t.face(), 0);
    assert_eq!(t.a_liberer(), None);
}

#[test]
fn ecrire_sans_avoir_recu_la_liberation_est_detecte() {
    let mut t = Tampons::double();
    t.trame_prete(0);
    t.trame_prete(1); // 0 a liberer, pas encore envoye
    assert_eq!(t.trame_prete(0), Issue::EcritureSansLiberation);
    assert_eq!(t.face(), 0);
    // L'ancienne face (1) est maintenant due ; la dette sur 0 a disparu
    // puisqu'il est redevenu face.
    assert_eq!(t.a_liberer(), Some(1));
    assert_eq!(t.etat(0), Some(Etat::Face));
}

#[test]
fn un_tampon_hors_surface_ne_change_rien() {
    let mut t = Tampons::double();
    t.trame_prete(1);
    let avant = t;
    assert_eq!(t.trame_prete(2), Issue::HorsBornes);
    assert_eq!(t.trame_prete(u32::MAX), Issue::HorsBornes);
    assert_eq!(t, avant);
}

/// L'invariant de securite, sur des entrelacements deterministes : un
/// client conforme n'ecrit jamais dans la face, quel que soit l'ordre dans
/// lequel le gestionnaire lit les trames et reussit (ou non) a envoyer les
/// liberations.
#[test]
fn un_client_conforme_n_ecrit_jamais_dans_la_face() {
    let mut graine: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut alea = move || {
        graine ^= graine << 13;
        graine ^= graine >> 7;
        graine ^= graine << 17;
        graine
    };
    for _ in 0..2_000 {
        let mut wm = Tampons::double();
        let mut client = Client::neuf(2);
        // Trames annoncees mais pas encore lues par le gestionnaire.
        let mut en_vol: Vec<u32> = Vec::new();
        // Liberations envoyees mais pas encore lues par le client.
        let mut liberations: Vec<u32> = Vec::new();
        for _ in 0..200 {
            match alea() % 4 {
                0 => {
                    // Le client peint une trame s'il a un tampon libre.
                    if let Some(t) = client.choisit() {
                        assert_ne!(wm.etat(t as usize), Some(Etat::Face), "le client ecrit dans la face");
                        assert_ne!(wm.etat(t as usize), Some(Etat::ALiberer), "le client ecrit un tampon non libere");
                        client.libres[t as usize] = false;
                        en_vol.push(t);
                    }
                }
                1 => {
                    // Le gestionnaire lit les trames en vol.
                    for t in en_vol.drain(..) {
                        assert_eq!(wm.trame_prete(t), Issue::Acceptee);
                    }
                }
                2 => {
                    // Le gestionnaire tente ses liberations ; le canal est
                    // plein une fois sur trois.
                    while let Some(t) = wm.a_liberer() {
                        if alea() % 3 == 0 {
                            break;
                        }
                        assert!(wm.liberation_envoyee(t));
                        liberations.push(t);
                    }
                }
                _ => {
                    // Le client lit ses liberations.
                    for t in liberations.drain(..) {
                        client.libres[t as usize] = true;
                    }
                }
            }
        }
    }
}

/// Vivacite : si le gestionnaire finit par vider son canal, le client
/// retrouve toujours un tampon libre -- personne n'attend une liberation
/// perdue.
#[test]
fn le_client_retrouve_toujours_un_tampon_libre() {
    let mut wm = Tampons::double();
    let mut client = Client::neuf(2);
    for trame in 0..1_000u32 {
        let t = client.choisit().expect("client bloque sans tampon libre");
        client.libres[t as usize] = false;
        assert_eq!(wm.trame_prete(t), Issue::Acceptee, "trame {trame}");
        while let Some(l) = wm.a_liberer() {
            assert!(wm.liberation_envoyee(l));
            client.libres[l as usize] = true;
        }
    }
}
