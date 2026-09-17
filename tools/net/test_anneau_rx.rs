//! L'anneau de reception RTL8168 tourne-t-il vraiment en rond ?
//!
//! # Le releve que ces tests defendent
//!
//! TRIGKEY, 17 septembre 2026, lien a un gigabit et cable branche :
//!
//! ```text
//! [NET-ROUTAGE] trames=104 arp=11 dhcp=2 arp_resolus=1 ... rx_abimees=0
//! ```
//!
//! Puis plus rien, pour toujours. L'emission continue, les requetes ARP
//! partent, et aucune reponse n'entre. La reception est morte apres cent
//! quatre trames.
//!
//! Tout ce qui pouvait produire cette signature -- un indice qui ne revient
//! pas a zero, un `EOR` pose deux fois, une longueur nulle rendue au
//! materiel, une adresse DMA deplacee, une trame abimee qui suspend le
//! drainage, un moteur arrete qu'on ne relance pas -- est ici de
//! l'arithmetique, et se contredit sans demarrer la machine.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

#[path = "../../src/drivers/network/anneau_rx.rs"]
mod anneau;

use anneau::{
    a_acquitter, adresse_tampon, carte_absente, cmd, cplus_cmd, degre, eor_pour, examine,
    generation, invariant_casse, isr, moteur_rx_a_relancer, nom, opts1_rendu, recense,
    reception_arretee, rx_config, suivant, xid, Degre, Generation, Recensement, Sante,
    Verdict, DESCRIPTEURS, EOR, ERR_CRC, ERR_RES, ERR_RUNT, ERR_RWT, MASQUE_LONGUEUR, OCTETS_FCS,
    OWN, RX128_INT_EN, RX_DMA_BURST, RX_EARLY_OFF, RX_FIFO_THRESH_HISTORIQUE, RX_MULTI_EN,
    TAILLE_TAMPON,
};

// ===========================================================================
// L'indice
// ===========================================================================

#[test]
fn l_indice_revient_a_zero_apres_le_dernier() {
    assert_eq!(suivant(62, 64), 63);
    assert_eq!(suivant(63, 64), 0);
    assert_eq!(suivant(0, 64), 1);
    assert_eq!(suivant(1, 64), 2);
}

#[test]
fn l_indice_fait_un_tour_complet_sans_sauter_personne() {
    let mut vus = vec![false; DESCRIPTEURS];
    let mut index = 0usize;
    for _ in 0..DESCRIPTEURS {
        assert!(!vus[index], "l'indice {index} est servi deux fois par tour");
        vus[index] = true;
        index = suivant(index, DESCRIPTEURS);
    }
    assert_eq!(index, 0, "un tour complet doit ramener a zero");
    assert!(vus.iter().all(|&v| v), "un descripteur n'est jamais servi");
}

#[test]
fn un_anneau_degenere_ne_divise_pas_par_zero() {
    assert_eq!(suivant(0, 0), 0);
    assert_eq!(eor_pour(0, 0), 0);
}

// ===========================================================================
// EOR, OWN, longueur, adresse
// ===========================================================================

#[test]
fn eor_n_appartient_qu_au_dernier_descripteur() {
    for index in 0..DESCRIPTEURS - 1 {
        assert_eq!(eor_pour(index, DESCRIPTEURS), 0, "EOR pose sur {index}");
    }
    assert_eq!(eor_pour(DESCRIPTEURS - 1, DESCRIPTEURS), EOR);
}

#[test]
fn rendre_un_descripteur_pose_own_eor_et_la_longueur() {
    let ordinaire = opts1_rendu(5, DESCRIPTEURS, TAILLE_TAMPON);
    assert_ne!(ordinaire & OWN, 0, "le materiel doit reprendre le descripteur");
    assert_eq!(ordinaire & EOR, 0);
    assert_eq!(ordinaire & MASQUE_LONGUEUR, TAILLE_TAMPON);

    let dernier = opts1_rendu(DESCRIPTEURS - 1, DESCRIPTEURS, TAILLE_TAMPON);
    assert_ne!(dernier & OWN, 0);
    assert_ne!(dernier & EOR, 0, "le dernier descripteur perd son EOR");
    assert_eq!(dernier & MASQUE_LONGUEUR, TAILLE_TAMPON);
}

#[test]
fn l_adresse_dma_suit_l_indice_et_rien_d_autre() {
    let base = 0x1_0000_0000u64;
    for index in 0..DESCRIPTEURS {
        assert_eq!(
            adresse_tampon(base, index, TAILLE_TAMPON),
            base + (index as u64) * u64::from(TAILLE_TAMPON)
        );
    }
    // Au-dela de quatre gibioctets : c'est le cas qui a motive les registres
    // d'adresse haute, et il ne doit pas se tronquer.
    assert!(adresse_tampon(base, 63, TAILLE_TAMPON) > u64::from(u32::MAX));
}

// ===========================================================================
// Le verdict d'une trame
// ===========================================================================

fn recu(longueur: u32) -> u32 {
    longueur & MASQUE_LONGUEUR
}

#[test]
fn une_trame_bonne_perd_son_fcs_et_rien_de_plus() {
    match examine(recu(1518), TAILLE_TAMPON) {
        Verdict::Bonne(n) => assert_eq!(n, 1518 - OCTETS_FCS),
        autre => panic!("{autre:?}"),
    }
    // Une requete ARP : 60 octets bourres + 4 de FCS.
    match examine(recu(64), TAILLE_TAMPON) {
        Verdict::Bonne(n) => assert_eq!(n, 60),
        autre => panic!("{autre:?}"),
    }
}

#[test]
fn un_descripteur_encore_au_materiel_ne_se_lit_pas() {
    assert_eq!(examine(OWN | recu(1518), TAILLE_TAMPON), Verdict::Materiel);
    // Meme avec des bits d'erreur : la propriete passe avant tout.
    assert_eq!(examine(OWN | ERR_CRC | recu(64), TAILLE_TAMPON), Verdict::Materiel);
}

#[test]
fn les_quatre_bits_d_erreur_condamnent_la_trame() {
    for (bit, nom) in [
        (ERR_RWT, "RWT"),
        (ERR_RES, "RES"),
        (ERR_RUNT, "RUNT"),
        (ERR_CRC, "CRC"),
    ] {
        assert_eq!(
            examine(bit | recu(1518), TAILLE_TAMPON),
            Verdict::Abimee,
            "{nom} n'est plus une erreur"
        );
    }
}

#[test]
fn une_longueur_impossible_est_une_trame_abimee() {
    // Plus courte que son propre FCS.
    assert_eq!(examine(recu(0), TAILLE_TAMPON), Verdict::Abimee);
    assert_eq!(examine(recu(4), TAILLE_TAMPON), Verdict::Abimee);
    // Plus longue que le tampon : la lire deborderait.
    assert_eq!(examine(recu(TAILLE_TAMPON + 1), TAILLE_TAMPON), Verdict::Abimee);
    // Exactement la taille du tampon reste lisible.
    assert!(matches!(examine(recu(TAILLE_TAMPON), TAILLE_TAMPON), Verdict::Bonne(_)));
}

// ===========================================================================
// Le drainage : un modele d'anneau, et ce qu'il doit rendre
// ===========================================================================

struct Anneau {
    opts1: Vec<u32>,
    adresses: Vec<u64>,
    base: u64,
    curseur: usize,
    rearmes: usize,
}

impl Anneau {
    fn neuf() -> Self {
        let base = 0x4000_0000u64;
        Self {
            opts1: (0..DESCRIPTEURS)
                .map(|i| opts1_rendu(i, DESCRIPTEURS, TAILLE_TAMPON))
                .collect(),
            adresses: (0..DESCRIPTEURS)
                .map(|i| adresse_tampon(base, i, TAILLE_TAMPON))
                .collect(),
            base,
            curseur: 0,
            rearmes: 0,
        }
    }

    /// Le materiel depose une trame dans le prochain descripteur qu'il possede.
    fn depose(&mut self, index: usize, opts1_sans_own: u32) {
        assert_ne!(self.opts1[index] & OWN, 0, "le materiel n'a pas ce descripteur");
        self.opts1[index] = opts1_sans_own;
    }

    /// Le drainage du pilote : la meme boucle que `receive`, en pur.
    fn draine(&mut self, maximum: usize) -> (Vec<usize>, usize) {
        let mut bonnes = Vec::new();
        let mut abimees = 0usize;
        for _ in 0..maximum {
            let index = self.curseur;
            match examine(self.opts1[index], TAILLE_TAMPON) {
                Verdict::Materiel => break,
                Verdict::Bonne(n) => bonnes.push(n),
                Verdict::Abimee => abimees += 1,
            }
            self.adresses[index] = adresse_tampon(self.base, index, TAILLE_TAMPON);
            self.opts1[index] = opts1_rendu(index, DESCRIPTEURS, TAILLE_TAMPON);
            self.rearmes += 1;
            self.curseur = suivant(index, DESCRIPTEURS);
        }
        (bonnes, abimees)
    }

    fn recensement(&self) -> Recensement {
        recense(DESCRIPTEURS, |i| self.opts1[i])
    }

    fn invariant(&self) -> Option<&'static str> {
        invariant_casse(
            DESCRIPTEURS,
            self.base,
            TAILLE_TAMPON,
            |i| self.opts1[i],
            |i| self.adresses[i],
        )
    }
}

#[test]
fn un_anneau_neuf_est_entierement_au_materiel() {
    let anneau = Anneau::neuf();
    assert_eq!(anneau.invariant(), None);
    let bilan = anneau.recensement();
    assert_eq!(bilan.materiel, DESCRIPTEURS);
    assert_eq!(bilan.processeur, 0);
    assert!(bilan.tout_au_materiel());
    assert!(!bilan.sature());
}

#[test]
fn plusieurs_trames_se_drainent_d_un_seul_coup() {
    let mut anneau = Anneau::neuf();
    for index in 0..5 {
        anneau.depose(index, recu(100 + index as u32));
    }
    let (bonnes, abimees) = anneau.draine(32);
    assert_eq!(bonnes.len(), 5, "le drainage s'arrete avant la fin");
    assert_eq!(abimees, 0);
    assert_eq!(bonnes[0], 100 - OCTETS_FCS);
    assert_eq!(bonnes[4], 104 - OCTETS_FCS);
    assert_eq!(anneau.curseur, 5);
    assert_eq!(anneau.invariant(), None, "le drainage casse l'anneau");
    assert!(anneau.recensement().tout_au_materiel());
}

#[test]
fn une_trame_abimee_ne_suspend_pas_celles_qui_suivent() {
    // C'EST LA PANNE QUI PERD UNE REPONSE ARP.
    //
    // Une reponse ARP est unique -- aucune retransmission de protocole. Si une
    // collision precede la reponse dans l'anneau et que le drainage s'arrete
    // sur elle, la resolution echoue et l'echec est mis en cache.
    let mut anneau = Anneau::neuf();
    anneau.depose(0, ERR_CRC | recu(64));
    anneau.depose(1, recu(64)); // la reponse ARP
    let (bonnes, abimees) = anneau.draine(32);
    assert_eq!(abimees, 1);
    assert_eq!(bonnes, vec![60], "la trame qui suit l'abimee est perdue");
}

#[test]
fn un_anneau_entierement_abime_rend_la_main() {
    let mut anneau = Anneau::neuf();
    for index in 0..DESCRIPTEURS {
        anneau.depose(index, ERR_RUNT | recu(64));
    }
    let (bonnes, abimees) = anneau.draine(DESCRIPTEURS);
    assert!(bonnes.is_empty());
    assert_eq!(abimees, DESCRIPTEURS);
    assert_eq!(anneau.curseur, 0, "un tour complet ramene a zero");
    assert_eq!(anneau.invariant(), None);
}

#[test]
fn le_drainage_passe_le_dernier_descripteur_sans_perdre_eor() {
    let mut anneau = Anneau::neuf();
    anneau.curseur = 62;
    for index in [62usize, 63, 0, 1] {
        anneau.depose(index, recu(64));
    }
    let (bonnes, _) = anneau.draine(8);
    assert_eq!(bonnes.len(), 4, "le passage 63 -> 0 perd des trames");
    assert_eq!(anneau.curseur, 2);
    assert_ne!(anneau.opts1[DESCRIPTEURS - 1] & EOR, 0, "EOR perdu au passage");
    assert_eq!(anneau.invariant(), None);
}

#[test]
fn un_anneau_plein_de_trames_non_drainees_est_sature() {
    let mut anneau = Anneau::neuf();
    for index in 0..DESCRIPTEURS {
        anneau.depose(index, recu(1518));
    }
    let bilan = anneau.recensement();
    assert_eq!(bilan.materiel, 0);
    assert_eq!(bilan.processeur, DESCRIPTEURS);
    assert!(bilan.sature(), "le materiel n'a plus ou ecrire, et rien ne le dit");

    // Le drainage complet le rend entierement au materiel.
    let (bonnes, _) = anneau.draine(DESCRIPTEURS);
    assert_eq!(bonnes.len(), DESCRIPTEURS);
    assert!(anneau.recensement().tout_au_materiel());
}

#[test]
fn un_drainage_borne_laisse_le_reste_pour_le_passage_suivant() {
    let mut anneau = Anneau::neuf();
    for index in 0..DESCRIPTEURS {
        anneau.depose(index, recu(64));
    }
    let (premier, _) = anneau.draine(32);
    assert_eq!(premier.len(), 32);
    let bilan = anneau.recensement();
    assert_eq!(bilan.processeur, DESCRIPTEURS - 32);
    let (second, _) = anneau.draine(32);
    assert_eq!(second.len(), 32, "le reste de l'anneau est perdu");
    assert!(anneau.recensement().tout_au_materiel());
}

// ===========================================================================
// L'invariant
// ===========================================================================

#[test]
fn un_eor_en_double_casse_l_invariant() {
    let mut anneau = Anneau::neuf();
    anneau.opts1[10] |= EOR;
    assert_eq!(anneau.invariant(), Some("EOR hors du dernier descripteur"));
}

#[test]
fn un_eor_absent_casse_l_invariant() {
    let mut anneau = Anneau::neuf();
    anneau.opts1[DESCRIPTEURS - 1] &= !EOR;
    assert_eq!(anneau.invariant(), Some("EOR absent ou en double"));
}

#[test]
fn une_adresse_dma_deplacee_casse_l_invariant() {
    let mut anneau = Anneau::neuf();
    anneau.adresses[7] += 16;
    assert_eq!(anneau.invariant(), Some("adresse DMA deplacee"));
}

#[test]
fn une_longueur_nulle_rendue_au_materiel_casse_l_invariant() {
    // Le materiel saute un descripteur de longueur nulle. L'anneau se vide
    // sans que rien ne le dise -- exactement la signature du releve.
    let mut anneau = Anneau::neuf();
    anneau.opts1[3] = OWN;
    assert_eq!(
        anneau.invariant(),
        Some("longueur rendue au materiel incorrecte")
    );
}

#[test]
fn un_descripteur_tenu_par_le_processeur_ne_casse_pas_l_invariant() {
    // Une trame en attente de drainage est NORMALE. La confondre avec une
    // rupture ferait reconstruire l'anneau a chaque paquet.
    let mut anneau = Anneau::neuf();
    anneau.depose(9, recu(1518));
    assert_eq!(anneau.invariant(), None);
}

// ===========================================================================
// Les statuts, en mode scrutation
// ===========================================================================

#[test]
fn une_carte_absente_ne_s_acquitte_pas() {
    assert!(carte_absente(0xFFFF));
    assert_eq!(a_acquitter(0xFFFF), 0, "acquitter dans le vide");
    assert!(!carte_absente(0x0001));
}

#[test]
fn l_acquittement_rend_exactement_ce_qui_a_ete_lu() {
    // Le choix contre celui d'U-Boot : voir `a_acquitter`. Un bit laisse
    // verrouille serait recompte a chaque passage.
    let lu = isr::RX_OK | isr::RX_ERR | isr::RX_FIFO_OVER | isr::SYS_ERR;
    assert_eq!(a_acquitter(lu), lu);
    assert_eq!(a_acquitter(0), 0, "rien a acquitter, rien a ecrire");
}

#[test]
fn le_moteur_arrete_se_reconnait_a_chip_cmd() {
    // Le cas NORMAL : reception et emission actives, tampons disponibles.
    assert!(!moteur_rx_a_relancer(cmd::RX_ENB | cmd::TX_ENB));
    // Le moteur est tombe. Personne dans le pilote ne l'arrete.
    assert!(moteur_rx_a_relancer(cmd::TX_ENB));
    // Le controleur declare ne plus avoir de tampon. Apres un drainage
    // complet, c'est faux -- et un bit qui ment est un moteur qui n'a pas
    // repris.
    assert!(moteur_rx_a_relancer(cmd::RX_ENB | cmd::TX_ENB | cmd::RX_BUF_EMPTY));
}

// ===========================================================================
// La revision du silicium
// ===========================================================================

#[test]
fn le_xid_se_lit_ou_linux_le_lit() {
    // `xid = (txconfig >> 20) & 0xfcf`, comme `rtl8169_init_one`.
    assert_eq!(xid(0x4c0 << 20), 0x4c0);
    assert_eq!(xid(0x3c0 << 20), 0x3c0);
    // Les bits hors masque ne doivent pas fuir dans l'identifiant.
    assert_eq!(xid(0xffff_ffff), 0xfcf);
}

#[test]
fn les_generations_suivent_la_table_de_linux() {
    assert_eq!(generation(0x4c0), Generation::ReceptionDifferee); // 8168g
    assert_eq!(generation(0x541), Generation::ReceptionDifferee); // 8168h
    assert_eq!(generation(0x5c8), Generation::ReceptionDifferee); // 8411b
    assert_eq!(generation(0x3c0), Generation::Multi); // 8168c
    assert_eq!(generation(0x2c0), Generation::Multi); // 8168e
    assert_eq!(generation(0x481), Generation::Multi); // 8168f
    assert_eq!(generation(0x380), Generation::Historique); // 8168b
    assert_eq!(generation(0x008), Generation::Historique); // 8169s
    assert_eq!(generation(0xabc), Generation::Inconnue);
}

#[test]
fn rx_config_ne_pose_plus_le_seuil_historique_sur_les_puces_modernes() {
    // LE DEFAUT : `7 << 13` etait ecrit pour TOUT LE MONDE. Sur 8111c et
    // suivants les bits 15 et 14 sont `RX128_INT_EN` et `RX_MULTI_EN`, et le
    // bit 13 doit rester nul.
    let moderne = rx_config(Generation::Multi);
    assert_eq!(moderne & (1 << 13), 0, "le bit 13 du seuil historique reste pose");
    assert_ne!(moderne & RX128_INT_EN, 0);
    assert_ne!(moderne & RX_MULTI_EN, 0);
    assert_ne!(moderne & RX_DMA_BURST, 0);

    // 8168g et suivants : la remise anticipee doit etre COUPEE.
    let recent = rx_config(Generation::ReceptionDifferee);
    assert_ne!(recent & RX_EARLY_OFF, 0, "RX_EARLY_OFF manquant sur 8168g+");
    assert_eq!(recent & (1 << 13), 0);

    // Et le seuil historique reste, pour les puces qui l'ont vraiment.
    assert_eq!(
        rx_config(Generation::Historique) & RX_FIFO_THRESH_HISTORIQUE,
        RX_FIFO_THRESH_HISTORIQUE
    );

    // Le defaut prudent de `rtl_init_rxcfg` pour une revision inconnue.
    let inconnue = rx_config(Generation::Inconnue);
    assert_eq!(inconnue, RX128_INT_EN | RX_DMA_BURST);
}

#[test]
fn rx_config_ne_porte_aucun_bit_d_acceptation() {
    // Les bits d'acceptation s'ecrivent APRES, comme `rtl_set_rx_mode`. Les
    // melanger ici ferait perdre la diffusion a chaque reprogrammation.
    for gen in [
        Generation::Historique,
        Generation::Multi,
        Generation::ReceptionDifferee,
        Generation::Inconnue,
    ] {
        assert_eq!(rx_config(gen) & anneau::MASQUE_ACCEPTATION, 0, "{}", nom(gen));
    }
}

#[test]
fn cplus_cmd_efface_pcidac_et_garde_ce_que_linux_garde() {
    // `tp->cp_cmd = RTL_R16(tp, CPlusCmd) & CPCMD_MASK`.
    let lu = anneau::CPCMD_PCIDAC | anneau::CPCMD_RX_CHKSUM | anneau::CPCMD_MODE_NORMAL | 0x0800;
    let ecrit = cplus_cmd(lu);
    assert_eq!(ecrit & anneau::CPCMD_PCIDAC, 0, "PCIDAC reste pose");
    assert_ne!(ecrit & anneau::CPCMD_RX_CHKSUM, 0);
    assert_ne!(ecrit & anneau::CPCMD_MODE_NORMAL, 0);
    assert_eq!(ecrit & 0x0800, 0, "un bit hors masque a survecu");
}

// ===========================================================================
// L'arret et sa reprise
// ===========================================================================

const SEC: u64 = 1_000_000_000;

#[test]
fn un_reseau_calme_n_est_pas_un_reseau_mort() {
    // Rien ne sort, rien n'entre : personne n'attend rien.
    let sante = Sante {
        lien: true,
        rx_dernier_ns: 10 * SEC,
        tx_dernier_ns: 0,
        tx_premier_ns: 0,
        reprise_derniere_ns: 0,
    };
    assert!(!reception_arretee(&sante, 60 * SEC));
}

#[test]
fn lien_bas_n_est_pas_une_panne_de_reception() {
    // Le cable est debranche : ce n'est pas au moteur RX de le reparer.
    let sante = Sante {
        lien: false,
        rx_dernier_ns: SEC,
        tx_dernier_ns: 9 * SEC,
        tx_premier_ns: SEC,
        reprise_derniere_ns: 0,
    };
    assert!(!reception_arretee(&sante, 10 * SEC));
}

#[test]
fn emettre_sans_jamais_rien_recevoir_est_une_panne() {
    // La signature EXACTE du releve : les requetes ARP partent, rien ne
    // revient, et cela dure.
    // La resolution ARP reemet toutes les cinq cents millisecondes : la
    // derniere emission est donc toujours fraiche, et le silence se compte
    // depuis la premiere.
    let sante = Sante {
        lien: true,
        rx_dernier_ns: 0,
        tx_dernier_ns: 23 * SEC + SEC / 2,
        tx_premier_ns: 20 * SEC,
        reprise_derniere_ns: 0,
    };
    assert!(reception_arretee(&sante, 24 * SEC));
    // Mais pas avant la borne : trois secondes de silence ne sont pas une
    // panne, et relancer le moteur perdrait la reponse en vol.
    assert!(!reception_arretee(&sante, 22 * SEC));
    // Et une emission qui s'est tue depuis longtemps ne demande plus rien.
    let muet = Sante { tx_dernier_ns: 20 * SEC, ..sante };
    assert!(!reception_arretee(&muet, 30 * SEC));
}

#[test]
fn la_reception_qui_progresse_n_est_jamais_declaree_morte() {
    let sante = Sante {
        lien: true,
        rx_dernier_ns: 23 * SEC,
        tx_dernier_ns: 23 * SEC,
        tx_premier_ns: SEC,
        reprise_derniere_ns: 0,
    };
    assert!(!reception_arretee(&sante, 24 * SEC));
}

#[test]
fn deux_reprises_ne_s_enchainent_pas_sans_repos() {
    let sante = Sante {
        lien: true,
        rx_dernier_ns: 0,
        tx_dernier_ns: 25 * SEC,
        tx_premier_ns: 20 * SEC,
        reprise_derniere_ns: 23 * SEC,
    };
    assert!(!reception_arretee(&sante, 24 * SEC), "reprise en boucle");
    assert!(reception_arretee(&sante, 26 * SEC));
}

#[test]
fn l_echelle_de_reprise_va_du_moins_au_plus_invasif() {
    let sain = Recensement { materiel: DESCRIPTEURS, processeur: 0 };
    let retenu = Recensement { materiel: 60, processeur: 4 };
    let actif = cmd::RX_ENB | cmd::TX_ENB;

    // Rien d'anormal dans l'anneau : on draine et on acquitte.
    assert_eq!(degre(None, sain, actif, 0), Degre::Draine);
    // Des descripteurs retenus : on les rend.
    assert_eq!(degre(None, retenu, actif, 0), Degre::Rearme);
    // Le moteur est tombe : on le relance, LUI SEUL.
    assert_eq!(degre(None, sain, cmd::TX_ENB, 0), Degre::RelanceRx);
    assert_eq!(degre(None, sain, actif | cmd::RX_BUF_EMPTY, 0), Degre::RelanceRx);
    // L'anneau ne veut plus rien dire : on le reconstruit.
    assert_eq!(
        degre(Some("EOR absent ou en double"), sain, actif, 0),
        Degre::ReconstruitAnneau
    );
}

#[test]
fn la_carte_n_est_reinitialisee_qu_en_dernier_recours() {
    let sain = Recensement { materiel: DESCRIPTEURS, processeur: 0 };
    let actif = cmd::RX_ENB | cmd::TX_ENB;
    // UNE reinitialisation coupe le lien et oblige a refaire DHCP. Elle ne se
    // paie qu'apres que les degres plus doux ont echoue.
    assert!(degre(None, sain, actif, 0) < Degre::ReinitialiseCarte);
    assert!(degre(None, sain, actif, 1) < Degre::ReinitialiseCarte);
    assert_eq!(degre(None, sain, actif, 2), Degre::ReconstruitAnneau);
    assert_eq!(degre(None, sain, actif, 3), Degre::ReinitialiseCarte);
}

#[test]
fn l_invariant_casse_passe_devant_le_moteur_arrete() {
    // Relancer le moteur sur un anneau incoherent ne repare rien : il ecrirait
    // dans un anneau que nous ne savons plus lire.
    let sain = Recensement { materiel: DESCRIPTEURS, processeur: 0 };
    assert_eq!(
        degre(Some("adresse DMA deplacee"), sain, cmd::TX_ENB, 0),
        Degre::ReconstruitAnneau
    );
}
