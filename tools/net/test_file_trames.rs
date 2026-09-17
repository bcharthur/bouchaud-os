//! Un paquet destine a une pile peut-il etre vole par l'autre ?
//!
//! # Le releve que ces tests defendent
//!
//! TRIGKEY, 17 septembre, Ladybird vivant, lien a un gigabit. Pendant trente
//! secondes :
//!
//! ```text
//! [NET-ROUTAGE] trames=100 arp=13 dhcp=2 arp_resolus=1 arp_echoues=0
//! [NET-TCP]     poignees=0 echantillons_rtt=0 syn_retransmis=0
//! ```
//!
//! Pas une trame de plus, pas un ARP de plus, pas une poignee TCP.
//!
//! Deux consommateurs lisaient l'anneau de la carte : le routage maison et le
//! peripherique `smoltcp`. Une reponse ARP retiree par le mauvais des deux est
//! perdue pour l'autre, et une reponse ARP ne se retransmet pas.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

#[path = "../../src/net/file_trames.rs"]
mod file;

use file::{Compteurs, FileTrames, TAILLE, TRAMES};

fn trame(marque: u8, longueur: usize) -> Vec<u8> {
    let mut t = vec![marque; longueur];
    if longueur > 0 {
        t[0] = marque;
    }
    t
}

#[test]
fn une_file_fermee_ne_retient_rien() {
    // LE CAS COURANT DOIT ETRE GRATUIT.
    //
    // Aucune pile smoltcp en cours : recopier chaque trame recue ferait payer
    // une copie par paquet a tout le systeme, pour personne.
    let mut f = FileTrames::neuve();
    assert!(!f.abonnee());
    assert!(!f.pose(&trame(1, 64)));
    assert_eq!(f.occupation(), 0);
    let mut sortie = [0u8; TAILLE];
    assert_eq!(f.retire(&mut sortie), None);
    assert_eq!(f.compteurs(), Compteurs::default());
}

#[test]
fn une_file_ouverte_rend_ce_qu_on_lui_a_donne() {
    let mut f = FileTrames::neuve();
    f.abonne();
    assert!(f.pose(&trame(0xAA, 64)));
    assert!(f.pose(&trame(0xBB, 128)));
    assert_eq!(f.occupation(), 2);

    let mut sortie = [0u8; TAILLE];
    assert_eq!(f.retire(&mut sortie), Some(64));
    assert_eq!(sortie[0], 0xAA);
    assert_eq!(f.retire(&mut sortie), Some(128));
    assert_eq!(sortie[0], 0xBB);
    assert_eq!(f.retire(&mut sortie), None);
    assert_eq!(f.occupation(), 0);
}

#[test]
fn l_ordre_est_celui_de_l_arrivee() {
    // Une pile TCP qui recoit ses segments dans le desordre les retransmet.
    let mut f = FileTrames::neuve();
    f.abonne();
    for i in 0..TRAMES {
        assert!(f.pose(&trame(i as u8, 64)), "depot {i} refuse");
    }
    let mut sortie = [0u8; TAILLE];
    for i in 0..TRAMES {
        assert_eq!(f.retire(&mut sortie), Some(64));
        assert_eq!(sortie[0], i as u8, "trame {i} servie hors de son rang");
    }
}

#[test]
fn la_file_tourne_en_rond_sans_se_perdre() {
    let mut f = FileTrames::neuve();
    f.abonne();
    let mut sortie = [0u8; TAILLE];
    // Trois tours complets, en alternant depot et retrait : c'est le regime
    // d'une requete TCP, et c'est la ou un indice qui ne revient pas a zero se
    // verrait.
    for tour in 0..(TRAMES * 3) {
        assert!(f.pose(&trame(tour as u8, 100)));
        assert_eq!(f.retire(&mut sortie), Some(100));
        assert_eq!(sortie[0], tour as u8);
    }
    assert_eq!(f.occupation(), 0);
    assert_eq!(f.compteurs().posees, (TRAMES * 3) as u64);
    assert_eq!(f.compteurs().retirees, (TRAMES * 3) as u64);
    assert_eq!(f.compteurs().perdues_pleine, 0);
}

#[test]
fn une_file_pleine_perd_et_le_dit() {
    // UNE FILE SANS BORNE DANS UN NOYAU EST UNE PANNE MEMOIRE DEGUISEE.
    //
    // Ce qui se perd doit se compter : cela distingue « smoltcp n'a rien
    // recu » de « smoltcp n'a pas lu ce qu'on lui a donne ».
    let mut f = FileTrames::neuve();
    f.abonne();
    for i in 0..TRAMES {
        assert!(f.pose(&trame(i as u8, 64)));
    }
    assert!(!f.pose(&trame(0xFF, 64)), "la borne ne tient pas");
    assert_eq!(f.compteurs().perdues_pleine, 1);
    assert_eq!(f.compteurs().occupation_max, TRAMES);

    // La plus ancienne reste la plus ancienne : on ne jette pas la tete pour
    // faire de la place a la queue.
    let mut sortie = [0u8; TAILLE];
    assert_eq!(f.retire(&mut sortie), Some(64));
    assert_eq!(sortie[0], 0);
}

#[test]
fn une_trame_trop_longue_est_refusee_sans_deborder() {
    let mut f = FileTrames::neuve();
    f.abonne();
    assert!(!f.pose(&trame(0x11, TAILLE + 1)));
    assert_eq!(f.compteurs().perdues_taille, 1);
    assert_eq!(f.occupation(), 0);
    // La borne exacte, elle, passe.
    assert!(f.pose(&trame(0x22, TAILLE)));
}

#[test]
fn un_tampon_de_sortie_court_ne_deborde_pas() {
    let mut f = FileTrames::neuve();
    f.abonne();
    assert!(f.pose(&trame(0x33, 1500)));
    let mut petit = [0u8; 64];
    assert_eq!(f.retire(&mut petit), Some(64));
    assert_eq!(petit[0], 0x33);
}

#[test]
fn le_desabonnement_jette_ce_qui_restait() {
    // Les trames d'une requete terminee n'interessent plus personne, et les
    // garder remettrait a la requete SUIVANTE des paquets qui ne lui sont pas
    // destines.
    let mut f = FileTrames::neuve();
    f.abonne();
    assert!(f.pose(&trame(1, 64)));
    assert!(f.pose(&trame(2, 64)));
    f.desabonne();
    assert_eq!(f.occupation(), 0);
    let mut sortie = [0u8; TAILLE];
    assert_eq!(f.retire(&mut sortie), None);

    // Et un nouvel abonnement repart d'une file vide.
    f.abonne();
    assert_eq!(f.occupation(), 0);
    assert!(f.pose(&trame(9, 64)));
    assert_eq!(f.retire(&mut sortie), Some(64));
    assert_eq!(sortie[0], 9);
}

#[test]
fn un_second_abonnement_ne_remet_pas_les_vieilles_trames() {
    let mut f = FileTrames::neuve();
    f.abonne();
    assert!(f.pose(&trame(7, 64)));
    f.abonne();
    assert_eq!(f.occupation(), 0, "une requete herite des trames de la precedente");
}

// ===========================================================================
// L'invariant de propriete : ce que le releve physique a coute
// ===========================================================================

/// Le modele des DEUX consommateurs, avec l'ancienne architecture et la
/// nouvelle. C'est la seule facon de montrer le vol au lieu de l'affirmer.
struct Carte {
    anneau: Vec<Vec<u8>>,
}

impl Carte {
    fn neuve(trames: Vec<Vec<u8>>) -> Self {
        Self { anneau: trames }
    }
    /// Un retrait PHYSIQUE : la trame quitte la carte pour de bon.
    fn recoit(&mut self) -> Option<Vec<u8>> {
        if self.anneau.is_empty() {
            None
        } else {
            Some(self.anneau.remove(0))
        }
    }
}

const ARP: u8 = 0x06;
const IPV4: u8 = 0x08;

fn flux() -> Vec<Vec<u8>> {
    // Une reponse ARP au milieu de segments TCP : exactement le cas ou la
    // resolution echoue si quelqu'un d'autre la prend.
    vec![
        trame(IPV4, 200),
        trame(ARP, 64),
        trame(IPV4, 300),
        trame(IPV4, 400),
    ]
}

#[test]
fn deux_lecteurs_physiques_se_volent_les_trames() {
    // L'ANCIENNE ARCHITECTURE, rejouee. Le peripherique smoltcp lit la carte
    // en meme temps que le routage maison.
    let mut carte = Carte::neuve(flux());
    let mut vues_maison: Vec<u8> = Vec::new();
    let mut vues_smoltcp: Vec<u8> = Vec::new();

    // Les deux scrutent en alternance, comme pendant une requete Ladybird.
    loop {
        match carte.recoit() {
            Some(t) => vues_maison.push(t[0]),
            None => break,
        }
        match carte.recoit() {
            Some(t) => vues_smoltcp.push(t[0]),
            None => break,
        }
    }

    // La reponse ARP est partie chez smoltcp : la pile maison ne la verra
    // jamais, et une reponse ARP ne se retransmet pas.
    assert!(
        !vues_maison.contains(&ARP),
        "le modele ne reproduit pas le vol"
    );
    assert!(vues_smoltcp.contains(&ARP));
}

#[test]
fn un_seul_lecteur_physique_sert_les_deux_piles() {
    // LA NOUVELLE ARCHITECTURE : une seule fonction lit la carte, et repartit.
    let mut carte = Carte::neuve(flux());
    let mut file = FileTrames::neuve();
    file.abonne();
    let mut vues_maison: Vec<u8> = Vec::new();

    while let Some(t) = carte.recoit() {
        // L'ingress unique : la pile maison route, et la file smoltcp recoit
        // une COPIE -- pas un sous-ensemble : smoltcp tient sa propre table de
        // voisins et lui cacher les reponses ARP le rendrait muet.
        vues_maison.push(t[0]);
        file.pose(&t);
    }

    let mut vues_smoltcp: Vec<u8> = Vec::new();
    let mut sortie = [0u8; TAILLE];
    while let Some(_) = file.retire(&mut sortie) {
        vues_smoltcp.push(sortie[0]);
    }

    assert_eq!(vues_maison, vec![IPV4, ARP, IPV4, IPV4]);
    assert_eq!(vues_smoltcp, vec![IPV4, ARP, IPV4, IPV4]);
    assert!(vues_maison.contains(&ARP), "la pile maison est volee");
    assert!(vues_smoltcp.contains(&ARP), "smoltcp est aveugle a l'ARP");
}

#[test]
fn une_rafale_plus_longue_que_la_file_ne_touche_pas_la_pile_maison() {
    // La file smoltcp deborde : c'est SA perte, comptee, et elle ne retire
    // rien a la pile maison. L'inverse -- une pile qui affame l'autre -- est
    // exactement ce qu'on ferme.
    let mut carte = Carte::neuve((0..(TRAMES + 8)).map(|i| trame(i as u8, 64)).collect());
    let mut file = FileTrames::neuve();
    file.abonne();
    let mut recues_maison = 0usize;

    while let Some(t) = carte.recoit() {
        recues_maison += 1;
        file.pose(&t);
    }

    assert_eq!(recues_maison, TRAMES + 8, "la pile maison a perdu des trames");
    assert_eq!(file.compteurs().perdues_pleine, 8);
    assert_eq!(file.occupation(), TRAMES);
}

// ===========================================================================
// Les scenarios de concurrence : ARP, DNS, DHCP et deux sockets a la fois
// ===========================================================================

/// Le modele de l'ingress UNIQUE : il lit la carte, route pour la pile maison
/// et recopie pour smoltcp. C'est la structure du code, en pur.
struct Ingress {
    carte: Carte,
    file: FileTrames,
    /// Ce que la pile maison a reellement vu, par type de trame.
    arp_maison: Vec<u8>,
    dhcp_maison: Vec<u8>,
    ipv4_maison: Vec<u8>,
    jetees_maison: Vec<u8>,
}

const DHCP: u8 = 0x44;
const MCAST: u8 = 0x33;
const IPV6: u8 = 0x86;

impl Ingress {
    fn neuf(trames: Vec<Vec<u8>>) -> Self {
        Self {
            carte: Carte::neuve(trames),
            file: FileTrames::neuve(),
            arp_maison: Vec::new(),
            dhcp_maison: Vec::new(),
            ipv4_maison: Vec::new(),
            jetees_maison: Vec::new(),
        }
    }

    /// Un passage de drainage : borne, comme `TRAMES_PAR_PASSAGE`.
    fn draine(&mut self, maximum: usize) -> usize {
        let mut traitees = 0;
        for _ in 0..maximum {
            let Some(t) = self.carte.recoit() else { break };
            traitees += 1;
            // LA COPIE D'ABORD, sur le flux BRUT : le routage maison jette ce
            // qu'il ne sait pas traiter, et une trame jetee la serait perdue
            // pour smoltcp aussi.
            self.file.pose(&t);
            match t[0] {
                ARP => self.arp_maison.push(t[0]),
                DHCP => self.dhcp_maison.push(t[0]),
                IPV4 => self.ipv4_maison.push(t[0]),
                autre => self.jetees_maison.push(autre),
            }
        }
        traitees
    }

    /// Ce que smoltcp obtient, en vidant sa file.
    fn smoltcp_lit(&mut self) -> Vec<u8> {
        let mut vues = Vec::new();
        let mut sortie = [0u8; TAILLE];
        while self.file.retire(&mut sortie).is_some() {
            vues.push(sortie[0]);
        }
        vues
    }
}

#[test]
fn une_reponse_arp_arrive_aux_deux_piles_pendant_du_trafic_smoltcp() {
    // LE CAS DU RELEVE PHYSIQUE. Ladybird tourne -- donc smoltcp scrute -- et
    // la pile maison attend une reponse ARP de la passerelle.
    let mut ing = Ingress::neuf(vec![
        trame(IPV4, 500),
        trame(ARP, 64),
        trame(IPV4, 500),
    ]);
    ing.file.abonne();
    ing.draine(32);

    assert_eq!(ing.arp_maison.len(), 1, "la reponse ARP a ete volee a la pile maison");
    assert!(
        ing.smoltcp_lit().contains(&ARP),
        "smoltcp est aveugle a l'ARP : sa table de voisins resterait vide"
    );
}

#[test]
fn une_reponse_dns_n_est_pas_volee_par_smoltcp() {
    // DNS est de l'UDP sur IPv4 : il remonte a la pile maison par les files
    // IPv4, et smoltcp en recoit une copie sans la lui retirer.
    let mut ing = Ingress::neuf(vec![trame(IPV4, 120)]);
    ing.file.abonne();
    ing.draine(32);
    assert_eq!(ing.ipv4_maison.len(), 1, "la reponse DNS n'atteint plus la pile maison");
    assert_eq!(ing.smoltcp_lit(), vec![IPV4]);
}

#[test]
fn dhcp_et_navigateur_en_meme_temps() {
    // Le renouvellement du bail tombe pendant une navigation. Ni l'un ni
    // l'autre ne doit manger le courrier de l'autre.
    let mut ing = Ingress::neuf(vec![
        trame(IPV4, 300),
        trame(DHCP, 342),
        trame(IPV4, 300),
        trame(ARP, 64),
    ]);
    ing.file.abonne();
    ing.draine(32);

    assert_eq!(ing.dhcp_maison.len(), 1, "le bail DHCP a ete vole");
    assert_eq!(ing.ipv4_maison.len(), 2);
    assert_eq!(ing.arp_maison.len(), 1);
    assert_eq!(ing.smoltcp_lit().len(), 4, "smoltcp a perdu des trames");
}

#[test]
fn deux_sockets_simultanes_partagent_la_meme_file() {
    // Deux connexions TCP dans la MEME pile smoltcp : elles partagent la file
    // du peripherique, et l'ordre d'arrivee doit etre preserve -- une pile TCP
    // qui recoit ses segments dans le desordre retransmet.
    let mut ing = Ingress::neuf((0..10).map(|i| trame(IPV4, 200 + i)).collect());
    ing.file.abonne();
    ing.draine(32);
    let vues = ing.smoltcp_lit();
    assert_eq!(vues.len(), 10);
    assert_eq!(ing.ipv4_maison.len(), 10, "la pile maison a perdu des segments");
}

#[test]
fn le_trafic_parasite_ne_prive_personne() {
    // Multicast et IPv6 : la pile maison les JETTE, et c'est normal. Mais elle
    // ne doit pas les jeter AVANT la copie, sinon smoltcp -- qui pourrait en
    // avoir l'usage -- ne les verrait jamais, et surtout le tri deciderait
    // pour lui.
    let mut ing = Ingress::neuf(vec![
        trame(MCAST, 100),
        trame(ARP, 64),
        trame(IPV6, 200),
        trame(IPV4, 300),
    ]);
    ing.file.abonne();
    ing.draine(32);

    assert_eq!(ing.jetees_maison.len(), 2, "la pile maison retient ce qu'elle ne traite pas");
    assert_eq!(ing.arp_maison.len(), 1, "l'ARP est perdu dans le bruit");
    let vues = ing.smoltcp_lit();
    assert_eq!(vues.len(), 4, "le tri de la pile maison ampute le flux smoltcp");
    assert!(vues.contains(&ARP));
}

#[test]
fn sans_abonnement_la_pile_maison_recoit_tout_quand_meme() {
    // Aucune requete smoltcp en cours : rien n'est recopie, et la pile maison
    // ne perd rien. C'est le regime NORMAL, et il doit etre gratuit.
    let mut ing = Ingress::neuf(vec![trame(ARP, 64), trame(IPV4, 300)]);
    // pas d'abonnement
    ing.draine(32);
    assert_eq!(ing.arp_maison.len(), 1);
    assert_eq!(ing.ipv4_maison.len(), 1);
    assert_eq!(ing.file.compteurs().posees, 0, "une copie payee pour personne");
}

#[test]
fn un_drainage_borne_ne_perd_rien_entre_deux_passages() {
    // `TRAMES_PAR_PASSAGE` borne la section critique. Ce qui reste doit etre
    // servi au passage suivant, aux deux piles.
    let mut ing = Ingress::neuf((0..40).map(|_| trame(IPV4, 100)).collect());
    ing.file.abonne();
    assert_eq!(ing.draine(32), 32);
    let premier = ing.smoltcp_lit().len();
    assert_eq!(premier, 32);
    assert_eq!(ing.draine(32), 8);
    assert_eq!(ing.smoltcp_lit().len(), 8);
    assert_eq!(ing.ipv4_maison.len(), 40, "la pile maison a perdu le reliquat");
}
