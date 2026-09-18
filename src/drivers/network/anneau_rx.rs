//! L'anneau de reception RTL8168 : sa DISCIPLINE, sans le materiel.
//!
//! # Pourquoi ce module existe separement
//!
//! Le releve physique TRIGKEY du 17 septembre montre un reseau qui vit deux
//! secondes puis meurt :
//!
//! ```text
//! [NET-LIEN]    verdict=pret lien=1 vitesse_mbps=1000 duplex=complet
//!               ip=192.168.1.97 gw=192.168.1.254 trames_perdues=0
//! [NET-ROUTAGE] trames=104 arp=11 dhcp=2 arp_resolus=1 ... rx_abimees=0
//! ```
//!
//! Les trois compteurs de routage ne bougent plus JAMAIS. Le lien est a un
//! gigabit, l'emission marche -- les requetes ARP partent, `arp_non_emis=0` --
//! et plus une seule trame n'entre. DNS rend alors `ENETUNREACH`, il n'y a
//! aucune poignee TCP, et Ladybird n'a rien a afficher.
//!
//! Tout ce qui decide de la vie de l'anneau -- l'indice suivant, le bit `EOR`,
//! la propriete `OWN`, la longueur rendue au materiel, l'adresse DMA, le
//! recensement des descripteurs, le verdict d'une trame, la politique
//! d'acquittement des statuts -- tenait dans du code `unsafe` derriere des
//! ecritures volatiles. Il n'etait donc verifiable que sur la machine, une
//! fois par flash. C'est exactement ce qui a laisse ce defaut vivre.
//!
//! Ici, la meme discipline est de l'arithmetique pure : un test hote la
//! contredit en une milliseconde.
//!
//! # Les references, et ce qu'on leur prend
//!
//! Le contrat suit `drivers/net/ethernet/realtek/r8169_main.c` (Linux) et
//! `drivers/net/rtl8169.c` (U-Boot). On ne COPIE pas Linux : il pilote par
//! interruption et NAPI, nous scrutons. On lui prend ce qui est une propriete
//! du SILICIUM -- la disposition des bits, la table des revisions, le choix de
//! `RxConfig` par generation -- et on prend a U-Boot ce qui est une propriete
//! du MODE SCRUTATION : acquitter `IntrStatus` a chaque passage a vide.

// ---------------------------------------------------------------------------
// Les descripteurs
// ---------------------------------------------------------------------------

/// Descripteurs de l'anneau de reception.
pub const DESCRIPTEURS: usize = 64;

/// Taille d'un tampon de reception, en octets.
///
/// Deux kibioctets : au-dessus d'une trame Ethernet normale (1518 avec le FCS)
/// et tres au-dessous de ce que le champ de longueur peut porter.
pub const TAILLE_TAMPON: u32 = 2048;

/// Le materiel possede ce descripteur.
pub const OWN: u32 = 1 << 31;
/// Dernier descripteur de l'anneau : le materiel revient a zero apres lui.
pub const EOR: u32 = 1 << 30;
/// Premier segment d'une trame.
pub const FS: u32 = 1 << 29;
/// Dernier segment d'une trame.
pub const LS: u32 = 1 << 28;

/// Chien de garde de reception expire (`RxRWT`).
pub const ERR_RWT: u32 = 1 << 22;
/// Resume d'erreur de reception (`RxRES`).
pub const ERR_RES: u32 = 1 << 21;
/// Trame trop courte (`RxRUNT`).
pub const ERR_RUNT: u32 = 1 << 20;
/// Somme de controle de trame fausse (`RxCRC`).
pub const ERR_CRC: u32 = 1 << 19;

/// Les quatre bits d'erreur, tels que r8169 et U-Boot les nomment.
pub const ERREURS: u32 = ERR_RWT | ERR_RES | ERR_RUNT | ERR_CRC;

/// Champ de longueur : quatorze bits.
pub const MASQUE_LONGUEUR: u32 = 0x3FFF;

/// Le controleur livre le FCS avec la trame ; il n'appartient pas a la pile.
pub const OCTETS_FCS: usize = 4;

/// L'indice suivant dans l'anneau.
///
/// Le seul endroit ou 63 devient 0. Une erreur ici ne se voit pas : l'anneau
/// continue de tourner, simplement decale du materiel, et la reception
/// s'arrete quelques trames plus tard sans rien dire.
#[inline]
pub fn suivant(index: usize, descripteurs: usize) -> usize {
    if descripteurs == 0 {
        return 0;
    }
    let apres = index + 1;
    if apres >= descripteurs {
        0
    } else {
        apres
    }
}

/// Le bit `EOR` appartient-il a ce descripteur ?
///
/// A UN SEUL, LE DERNIER. Le poser sur un autre ferait revenir le materiel a
/// zero trop tot -- il n'utiliserait plus qu'une partie de l'anneau, et les
/// descripteurs au-dela ne seraient jamais remplis. L'oublier sur le dernier
/// le ferait sortir de l'anneau et ecrire dans la memoire qui suit.
#[inline]
pub fn eor_pour(index: usize, descripteurs: usize) -> u32 {
    if descripteurs != 0 && index + 1 == descripteurs {
        EOR
    } else {
        0
    }
}

/// Le mot `opts1` a ecrire pour RENDRE un descripteur au materiel.
///
/// Trois choses ensemble, et il faut les trois : la propriete, le bit de fin
/// d'anneau, et la longueur DISPONIBLE du tampon. Rendre un descripteur avec
/// une longueur nulle le rend inutilisable sans que rien ne le signale ; le
/// materiel le saute et l'anneau se vide silencieusement.
#[inline]
pub fn opts1_rendu(index: usize, descripteurs: usize, taille_tampon: u32) -> u32 {
    OWN | eor_pour(index, descripteurs) | (taille_tampon & MASQUE_LONGUEUR)
}

/// L'adresse physique du tampon de ce descripteur.
#[inline]
pub fn adresse_tampon(base: u64, index: usize, taille_tampon: u32) -> u64 {
    base.wrapping_add((index as u64).wrapping_mul(u64::from(taille_tampon)))
}

/// Ce que dit le mot `opts1` rendu par le materiel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Le materiel possede encore ce descripteur : rien a lire ICI.
    ///
    /// « Rien a lire ici » n'est pas « rien a lire ». C'est la nuance qui
    /// manquait : voir `Recensement`.
    Materiel,
    /// Une trame utilisable, de cette longueur, FCS deja retire.
    Bonne(usize),
    /// Une trame a jeter. Elle est comptee, et la lecture CONTINUE : une seule
    /// trame abimee ne doit pas suspendre le drainage de celles qui la
    /// suivent.
    Abimee,
}

/// Examine un descripteur rendu par le materiel.
pub fn examine(opts1: u32, taille_tampon: u32) -> Verdict {
    if opts1 & OWN != 0 {
        return Verdict::Materiel;
    }
    let brute = (opts1 & MASQUE_LONGUEUR) as usize;
    if opts1 & ERREURS != 0 {
        return Verdict::Abimee;
    }
    // Une trame plus courte que son FCS n'a pas de charge utile ; une trame
    // plus longue que le tampon n'a pas pu y tenir, et la lire deborderait.
    if brute <= OCTETS_FCS || brute > taille_tampon as usize {
        return Verdict::Abimee;
    }
    Verdict::Bonne(brute - OCTETS_FCS)
}

// ---------------------------------------------------------------------------
// Le recensement : la question que le releve physique ne savait pas poser
// ---------------------------------------------------------------------------

/// Qui possede quoi dans l'anneau.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Recensement {
    /// Descripteurs rendus au materiel : il peut y ecrire.
    pub materiel: usize,
    /// Descripteurs que le processeur n'a pas encore rendus.
    pub processeur: usize,
}

impl Recensement {
    /// L'anneau est-il entierement rendu au materiel ?
    ///
    /// C'est l'etat NORMAL au repos. Le voir avec une reception figee dit que
    /// le materiel a de la place et n'ecrit pas : le moteur est arrete, ce
    /// n'est pas une question de tampons.
    #[inline]
    pub fn tout_au_materiel(&self) -> bool {
        self.processeur == 0 && self.materiel != 0
    }

    /// Le processeur retient-il TOUT ?
    ///
    /// L'anneau est plein de trames non drainees : le materiel n'a plus ou
    /// ecrire. C'est l'autre panne, et elle demande l'inverse -- drainer, pas
    /// relancer.
    #[inline]
    pub fn sature(&self) -> bool {
        self.materiel == 0
    }
}

/// Recense l'anneau. `opts1` lit le mot d'etat du descripteur donne.
pub fn recense(descripteurs: usize, opts1: impl Fn(usize) -> u32) -> Recensement {
    let mut bilan = Recensement::default();
    for index in 0..descripteurs {
        if opts1(index) & OWN != 0 {
            bilan.materiel += 1;
        } else {
            bilan.processeur += 1;
        }
    }
    bilan
}

/// L'invariant de l'anneau est-il intact ?
///
/// Rend le nom de ce qui est casse, ou `None`. C'est ce qui decide entre
/// « relancer le moteur » et « reconstruire l'anneau » dans l'echelle de
/// reprise : reconstruire un anneau sain perd les trames en vol pour rien.
pub fn invariant_casse(
    descripteurs: usize,
    base_tampons: u64,
    taille_tampon: u32,
    opts1: impl Fn(usize) -> u32,
    adresse: impl Fn(usize) -> u64,
) -> Option<&'static str> {
    if descripteurs == 0 {
        return Some("anneau vide");
    }
    let mut fins = 0usize;
    for index in 0..descripteurs {
        let mot = opts1(index);
        if mot & EOR != 0 {
            fins += 1;
            if index + 1 != descripteurs {
                return Some("EOR hors du dernier descripteur");
            }
        }
        if adresse(index) != adresse_tampon(base_tampons, index, taille_tampon) {
            return Some("adresse DMA deplacee");
        }
        // Un descripteur RENDU au materiel doit porter la longueur du tampon.
        // Zero est le cas qui compte : le materiel saute le descripteur et
        // l'anneau se vide sans que rien ne le dise.
        if mot & OWN != 0 && (mot & MASQUE_LONGUEUR) != (taille_tampon & MASQUE_LONGUEUR) {
            return Some("longueur rendue au materiel incorrecte");
        }
    }
    if fins != 1 {
        return Some("EOR absent ou en double");
    }
    None
}

// ---------------------------------------------------------------------------
// Les statuts d'interruption, en mode SCRUTATION
// ---------------------------------------------------------------------------

/// Bits de `IntrStatus` (0x3E), nommes comme dans r8169 et U-Boot.
pub mod isr {
    pub const RX_OK: u16 = 0x0001;
    pub const RX_ERR: u16 = 0x0002;
    pub const TX_OK: u16 = 0x0004;
    pub const TX_ERR: u16 = 0x0008;
    /// Plus de descripteur de reception disponible.
    pub const RX_OVERFLOW: u16 = 0x0010;
    pub const LINK_CHG: u16 = 0x0020;
    /// Debordement de la file d'attente interne de reception.
    pub const RX_FIFO_OVER: u16 = 0x0040;
    pub const TX_DESC_UNAVAIL: u16 = 0x0080;
    pub const SW_INT: u16 = 0x0100;
    pub const PCS_TIMEOUT: u16 = 0x4000;
    pub const SYS_ERR: u16 = 0x8000;
}

/// Bits de `ChipCmd` (0x37).
pub mod cmd {
    pub const STOP_REQ: u8 = 0x80;
    pub const RESET: u8 = 0x10;
    /// Moteur de reception actif.
    pub const RX_ENB: u8 = 0x08;
    /// Moteur d'emission actif.
    pub const TX_ENB: u8 = 0x04;
    /// Le controleur n'a plus de tampon de reception.
    ///
    /// C'est un bit de `ChipCmd`, et non de `IntrStatus` : il se lit meme sans
    /// interruption, et c'est exactement ce qu'il faut a un pilote qui scrute.
    pub const RX_BUF_EMPTY: u8 = 0x01;
}

/// Une lecture de `IntrStatus` designe-t-elle une carte ABSENTE ?
///
/// Tous les bits a un, c'est ce que rend un espace de configuration qui ne
/// repond plus -- la carte a disparu du bus, ou elle est dans un etat
/// d'economie d'energie. Acquitter dans ce cas ecrirait dans le vide, et
/// compter les bits raconterait une panne qui n'existe pas.
///
/// `rtl8169_interrupt` pose exactement ce test avant toute autre chose.
#[inline]
pub fn carte_absente(status: u16) -> bool {
    status == 0xFFFF
}

/// Ce qu'il faut REECRIRE dans `IntrStatus` pour acquitter ce qu'on a lu.
///
/// # Pourquoi tout, et pas la selection d'U-Boot
///
/// U-Boot, dans son chemin de scrutation, ecrit `sts & ~(TxErr|RxErr|SYSErr)`
/// : il laisse ces trois-la verrouilles pour pouvoir les relire plus tard.
/// C'est le bon choix pour un chargeur d'amorcage, qui n'a pas de compteurs et
/// qui vit quelques secondes.
///
/// Nous vivons des heures et nous COMPTONS. Un bit laisse verrouille serait
/// relu et recompte a chaque passage : `isr_rx_err` monterait de mille par
/// seconde pour une seule erreur, et le releve physique deviendrait illisible
/// au moment precis ou il doit servir. Linux acquitte tout ce qu'il a lu
/// (`rtl_ack_events(tp, status)`) pour cette raison-la.
///
/// L'information qu'U-Boot garde dans le registre, nous la gardons dans les
/// compteurs. Rien ne se perd, et chaque evenement compte une fois.
#[inline]
pub fn a_acquitter(status: u16) -> u16 {
    if carte_absente(status) || status == 0 {
        0
    } else {
        status
    }
}

/// Le moteur de reception a-t-il besoin qu'on le relance ?
///
/// Deux signes, et ils ne disent pas la meme chose :
///
///   * `RxEnb` tombe : le moteur est ARRETE. Personne dans notre pilote ne
///     l'arrete ; s'il est tombe, c'est le materiel qui l'a fait ;
///   * `RxBufEmpty` : le controleur declare n'avoir plus de tampon. Apres un
///     drainage complet c'est faux par construction, et un bit qui ment est un
///     moteur qui n'a pas repris.
#[inline]
pub fn moteur_rx_a_relancer(chip_cmd: u8) -> bool {
    chip_cmd & cmd::RX_ENB == 0 || chip_cmd & cmd::RX_BUF_EMPTY != 0
}

// ---------------------------------------------------------------------------
// La revision du silicium, et ce qu'elle change
// ---------------------------------------------------------------------------

/// Les generations que `RxConfig` distingue reellement.
///
/// Ce n'est pas la table des soixante revisions de Linux : c'est la seule
/// chose dont NOTRE pilote a besoin. Chaque variante correspond a une ligne de
/// `rtl_init_rxcfg`, et pas une de plus.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Generation {
    /// RTL8169/8110 et premiers 8168b : seuils de file d'attente historiques.
    Historique,
    /// 8168c/8168d/8168e/8411 : `RX128_INT_EN | RX_MULTI_EN`.
    Multi,
    /// 8168g/8168h/8411b/8168ep/8117 : la meme chose plus `RX_EARLY_OFF`.
    ReceptionDifferee,
    /// Revision non reconnue : le defaut prudent de Linux.
    Inconnue,
}

/// `RX128_INT_EN` -- 8111c et suivants.
pub const RX128_INT_EN: u32 = 1 << 15;
/// `RX_MULTI_EN` -- 8111c seulement, inoffensif ailleurs dans la meme plage.
pub const RX_MULTI_EN: u32 = 1 << 14;
/// Le seuil de file d'attente HISTORIQUE. Sur 8111c et suivants ces trois bits
/// ne sont plus un seuil : 15 et 14 ont un sens propre, et 13 doit rester nul.
pub const RX_FIFO_THRESH_HISTORIQUE: u32 = 7 << 13;
/// `RX_EARLY_OFF` -- coupe la remise anticipee sur 8168g et suivants.
pub const RX_EARLY_OFF: u32 = 1 << 11;
/// Rafale PCI de reception sans limite.
pub const RX_DMA_BURST: u32 = 7 << 8;

/// Les quatre bits d'acceptation, dans l'ordre du manuel.
pub const ACCEPTE_ERREURS: u32 = 0x20;
pub const ACCEPTE_RUNT: u32 = 0x10;
pub const ACCEPTE_DIFFUSION: u32 = 0x08;
pub const ACCEPTE_MULTIDIFFUSION: u32 = 0x04;
pub const ACCEPTE_MA_MAC: u32 = 0x02;
pub const ACCEPTE_TOUT: u32 = 0x01;
/// Masque des bits d'acceptation, pour les reecrire sans toucher au reste.
pub const MASQUE_ACCEPTATION: u32 = 0x0F;

/// L'identifiant de revision, extrait de `TxConfig`.
///
/// `xid = (txconfig >> 20) & 0xfcf`, exactement comme `rtl8169_init_one`.
#[inline]
pub fn xid(txconfig: u32) -> u32 {
    (txconfig >> 20) & 0xfcf
}

/// La generation correspondant a un `xid`.
///
/// La table est celle de `rtl_chip_infos`, reduite aux entrees qui changent
/// `RxConfig`. Les revisions sont rangees du plus recent au plus ancien, comme
/// chez Linux : les masques se chevauchent, et l'ordre EST la regle.
pub fn generation(xid: u32) -> Generation {
    const TABLE: &[(u32, u32, Generation)] = &[
        // 8168g / 8411b / 8168h / 8168ep / 8117 : reception differee coupee.
        (0x7cf, 0x4c0, Generation::ReceptionDifferee), // RTL8168g/8111g
        (0x7cf, 0x509, Generation::ReceptionDifferee), // RTL8168gu/8111gu
        (0x7cf, 0x5c8, Generation::ReceptionDifferee), // RTL8411b
        (0x7cf, 0x541, Generation::ReceptionDifferee), // RTL8168h/8111h
        (0x7cf, 0x6c0, Generation::ReceptionDifferee), // RTL8168M
        (0x7cf, 0x502, Generation::ReceptionDifferee), // RTL8168ep/8111ep
        (0x7cf, 0x54a, Generation::ReceptionDifferee), // RTL8168fp/8117
        (0x7cf, 0x54b, Generation::ReceptionDifferee), // RTL8168fp/8117
        // 8168c / 8168d / 8168e / 8411 : multi-descripteur.
        (0x7c8, 0x488, Generation::Multi), // RTL8411
        (0x7cf, 0x481, Generation::Multi), // RTL8168f/8111f
        (0x7cf, 0x480, Generation::Multi), // RTL8168f/8111f
        (0x7c8, 0x2c8, Generation::Multi), // RTL8168evl/8111evl
        (0x7cf, 0x2c1, Generation::Multi), // RTL8168e/8111e
        (0x7c8, 0x2c0, Generation::Multi), // RTL8168e/8111e
        (0x7cf, 0x281, Generation::Multi), // RTL8168d/8111d
        (0x7c8, 0x280, Generation::Multi), // RTL8168d/8111d
        (0x7cf, 0x28a, Generation::Multi), // RTL8168dp/8111dp
        (0x7cf, 0x28b, Generation::Multi), // RTL8168dp/8111dp
        (0x7cf, 0x3c9, Generation::Multi), // RTL8168cp/8111cp
        (0x7cf, 0x3c8, Generation::Multi), // RTL8168cp/8111cp
        (0x7c8, 0x3c8, Generation::Multi), // RTL8168cp/8111cp
        (0x7cf, 0x3c0, Generation::Multi), // RTL8168c/8111c
        (0x7cf, 0x3c2, Generation::Multi), // RTL8168c/8111c
        (0x7cf, 0x3c3, Generation::Multi), // RTL8168c/8111c
        (0x7c8, 0x3c0, Generation::Multi), // RTL8168c/8111c
        // 8168b et 8169 : le seuil historique.
        (0x7c8, 0x380, Generation::Historique), // RTL8168b/8111b
        (0x7c8, 0x300, Generation::Historique), // RTL8168b/8111b
        (0xfc8, 0x980, Generation::Historique), // RTL8169sc/8110sc
        (0xfc8, 0x180, Generation::Historique), // RTL8169sc/8110sc
        (0xfc8, 0x100, Generation::Historique), // RTL8169sb/8110sb
        (0xfc8, 0x040, Generation::Historique), // RTL8110s
        (0xfc8, 0x008, Generation::Historique), // RTL8169s
    ];
    let mut i = 0;
    while i < TABLE.len() {
        let (masque, valeur, gen) = TABLE[i];
        if xid & masque == valeur {
            return gen;
        }
        i += 1;
    }
    Generation::Inconnue
}

/// Le `RxConfig` d'une generation, bits d'acceptation NON compris.
///
/// # Le defaut que cette fonction ferme
///
/// Le pilote ecrivait `7 << 13` pour tout le monde, sous le nom
/// « RX_FIFO_THRESH ». C'est juste sur un RTL8169 de 2003. Sur 8111c et
/// suivants -- c'est-a-dire sur toute carte posee dans une machine de cette
/// decennie -- les bits 15 et 14 ne sont plus un seuil : ce sont
/// `RX128_INT_EN` et `RX_MULTI_EN`, et le bit 13 doit rester NUL. Nous
/// ecrivions donc les deux bons bits par accident, plus un troisieme que
/// Linux n'ecrit sur aucune revision moderne.
///
/// Et sur 8168g et suivants il manquait `RX_EARLY_OFF`, que `rtl_init_rxcfg`
/// pose sur toute la plage VER_40..VER_52.
pub fn rx_config(generation: Generation) -> u32 {
    match generation {
        Generation::Historique => RX_FIFO_THRESH_HISTORIQUE | RX_DMA_BURST,
        Generation::Multi => RX128_INT_EN | RX_MULTI_EN | RX_DMA_BURST,
        Generation::ReceptionDifferee => {
            RX128_INT_EN | RX_MULTI_EN | RX_DMA_BURST | RX_EARLY_OFF
        }
        // Le defaut de `rtl_init_rxcfg` : `RX128_INT_EN | RX_DMA_BURST`.
        Generation::Inconnue => RX128_INT_EN | RX_DMA_BURST,
    }
}

/// Le nom de la generation, pour le releve.
pub fn nom(generation: Generation) -> &'static str {
    match generation {
        Generation::Historique => "8169/8168b",
        Generation::Multi => "8168c-f",
        Generation::ReceptionDifferee => "8168g+",
        Generation::Inconnue => "inconnue",
    }
}

// ---------------------------------------------------------------------------
// `CPlusCmd`, et ce qu'il ne faut PAS y laisser
// ---------------------------------------------------------------------------

/// `Normal_mode`.
pub const CPCMD_MODE_NORMAL: u16 = 1 << 13;
/// Retrait materiel des etiquettes VLAN.
pub const CPCMD_RX_VLAN: u16 = 1 << 6;
/// Verification materielle des sommes de controle.
pub const CPCMD_RX_CHKSUM: u16 = 1 << 5;
/// Temporisation d'interruption -- sans objet en scrutation, mais preservee.
pub const CPCMD_INTT: u16 = 0x0003;
/// Cycle d'adressage double PCI.
pub const CPCMD_PCIDAC: u16 = 1 << 4;

/// Ce que Linux CONSERVE de `CPlusCmd` : `CPCMD_MASK`.
pub const CPCMD_MASQUE: u16 = CPCMD_MODE_NORMAL | CPCMD_RX_VLAN | CPCMD_RX_CHKSUM | CPCMD_INTT;

/// La valeur a ecrire dans `CPlusCmd` a partir de celle qu'on y a lue.
///
/// # `PCIDAC` etait pose, et Linux l'efface
///
/// Le pilote faisait `read16(CPlusCmd) | PCIDAC` pour pouvoir adresser des
/// anneaux au-dessus de quatre gibioctets. Sur un RTL8168 ce n'est pas ainsi
/// qu'on y accede : les registres `RxDescAddrHigh` et `TxDescAddrHigh`
/// portent les trente-deux bits hauts, et `rtl_init_one` ne garde de
/// `CPlusCmd` que `CPCMD_MASK` -- ou `PCIDAC` ne figure pas. Linux active
/// pourtant le DMA 64 bits sur toute revision >= VER_18.
///
/// Poser un bit que la reference efface sur une famille entiere, c'est
/// programmer un mode que personne ne teste.
#[inline]
pub fn cplus_cmd(lu: u16) -> u16 {
    lu & CPCMD_MASQUE
}

// ---------------------------------------------------------------------------
// La detection d'arret
// ---------------------------------------------------------------------------

/// Ce qu'il faut savoir pour dire « la reception est morte ».
#[derive(Clone, Copy, Debug, Default)]
pub struct Sante {
    /// Le lien est-il monte ?
    pub lien: bool,
    /// Instant de la derniere trame RECUE, en nanosecondes. Zero : jamais.
    pub rx_dernier_ns: u64,
    /// Instant de la derniere trame EMISE. Dit si nous PARLONS encore.
    pub tx_dernier_ns: u64,
    /// Instant de la PREMIERE trame emise.
    ///
    /// # Pourquoi deux instants d'emission, et non un seul
    ///
    /// Quand rien n'a JAMAIS ete recu, il faut bien compter le silence depuis
    /// quelque chose. Le compter depuis la DERNIERE emission ne marche pas :
    /// une resolution ARP reemet toutes les cinq cents millisecondes, la
    /// derniere emission est donc toujours toute fraiche, et l'arret ne serait
    /// jamais declare -- precisement dans le cas que le releve physique
    /// montre.
    ///
    /// Le silence se compte donc depuis l'instant ou nous avons COMMENCE a
    /// parler, et la fraicheur de la derniere emission sert a autre chose :
    /// verifier que nous parlons encore.
    pub tx_premier_ns: u64,
    /// Instant de la derniere reprise tentee.
    pub reprise_derniere_ns: u64,
}

/// Silence de reception tolere avant de conclure a un arret, en nanosecondes.
///
/// # Pourquoi trois secondes, et pas trois cents millisecondes
///
/// Un reseau domestique au repos peut rester une seconde sans une seule trame
/// de diffusion. Conclure trop vite ferait relancer le moteur pour rien, et
/// relancer le moteur perd les trames en vol -- dont, precisement, la reponse
/// ARP qu'on attend.
///
/// Trois secondes, avec la condition que l'EMISSION ait continue pendant ce
/// temps : ce n'est plus « le reseau est calme », c'est « nous parlons et
/// personne ne nous repond plus, alors que le cable est branche ».
pub const SILENCE_RX_NS: u64 = 3_000_000_000;

/// Duree pendant laquelle une emission reste EN ATTENTE DE REPONSE.
///
/// Elle borne la fenetre ou le chien de garde a le droit d'accuser la carte.
/// Elle doit etre plus longue que tout ce qui peut retarder son interrogation
/// -- au premier rang, le budget d'attente du client DHCP, quatre secondes,
/// qui bloque le fil du veilleur juste apres l'emission.
pub const ATTENTE_REPONSE_NS: u64 = 30_000_000_000;

// ---------------------------------------------------------------------------
// LES REGLAGES QUE LE 8168h EXIGE, ET QUE NOTRE MINIMALISME OMETTAIT
// ---------------------------------------------------------------------------
//
// # L'audit, XID par XID
//
// Le releve physique donne `xid=0x541`, soit RTL8168h/8111h -- ce que Linux
// appelle `RTL_GIGA_MAC_VER_46`. Notre pilote programmait `RxConfig`,
// `CPlusCmd`, les adresses d'anneaux et les filtres, et RIEN d'autre. Voici
// ce que `rtl_hw_start_8168h_1` fait en plus, et ce qu'on en retient :
//
// | registre        | Bouchaud | Linux (8168h)              | necessaire |
// |-----------------|----------|----------------------------|------------|
// | RxConfig        | pose     | `rtl_init_rxcfg` + early off | deja fait |
// | CPlusCmd        | pose     | quirks + PCIDAC efface     | deja fait  |
// | Rx/TxDescAddr   | pose     | `rtl_set_rx_tx_desc_registers` | deja fait |
// | MAR0/4, accept  | pose     | `rtl_set_rx_mode`          | deja fait  |
// | Config2 bit 7   | INTACT   | `ClkReqEn` efface          | OUI        |
// | Config5 bit 0   | INTACT   | `ASPM_en` efface           | OUI        |
// | MISC 13:14      | INTACT   | `pcie_state_l2l3_disable`  | OUI        |
// | MISC bit 19     | INTACT   | `rtl_disable_rxdvgate`     | OUI        |
// | ERI 0xC8/0xE8   | INTACT   | seuils FIFO                | non prouve |
// | EPHY            | INTACT   | tables par revision        | non prouve |
//
// # Pourquoi ces quatre-la, et pas les autres
//
// ASPM et CLKREQ laissent le lien PCIe descendre en L1 quand il est calme.
// Le MAC continue alors de recevoir et de lever `RxOK` -- il a ses propres
// tampons -- pendant que son moteur DMA ne peut plus atteindre la memoire de
// l'hote. La signature est exactement celle du releve : trafic normal tant
// qu'il est soutenu, puis plus un descripteur rendu, `RxEnb` toujours arme,
// `RxMissed` a zero, aucune erreur.
//
// C'est aussi l'un des defauts les plus anciens de cette famille de cartes
// chez Linux, et `rtl_hw_aspm_clkreq_enable(tp, false)` est ce que le pilote
// de reference fait avant de demarrer le materiel.
//
// `RXDV_GATE` et `l2l3` sont du meme ordre : ils coupent l'alimentation du
// chemin de reception. Les quatre ne font que DESACTIVER des economies
// d'energie ; aucun ne change le format des descripteurs ni le protocole.
//
// Les tables ERI et EPHY ne sont PAS ajoutees : elles varient par revision de
// silicium, et rien dans le releve ne les met en cause. On n'ajoute que ce
// qu'on peut justifier.

/// `Config2` : le bit d'autorisation `CLKREQ`.
pub const CONFIG2_CLKREQ: u8 = 1 << 7;
/// `Config5` : le bit d'autorisation ASPM.
pub const CONFIG5_ASPM: u8 = 1 << 0;
/// `MISC` : la porte du chemin de reception.
pub const MISC_RXDV_GATE: u32 = 1 << 19;
/// `MISC` : les etats L2/L3 du lien PCIe.
pub const MISC_L2L3: u32 = (1 << 14) | (1 << 13);

/// Cette generation demande-t-elle qu'on lui coupe ses economies d'energie ?
///
/// Vrai pour 8168g et suivants -- la famille qui porte `RX_EARLY_OFF`, donc
/// celle du XID `0x541` releve sur la TRIGKEY.
pub fn coupe_les_economies(generation: Generation) -> bool {
    matches!(generation, Generation::ReceptionDifferee)
}

/// `Config2` apres extinction de `CLKREQ`.
pub fn config2_sans_clkreq(actuel: u8) -> u8 {
    actuel & !CONFIG2_CLKREQ
}

/// `Config5` apres extinction d'ASPM.
pub fn config5_sans_aspm(actuel: u8) -> u8 {
    actuel & !CONFIG5_ASPM
}

/// `MISC` apres ouverture de la porte RX et extinction de L2/L3.
pub fn misc_chemin_rx_ouvert(actuel: u32) -> u32 {
    actuel & !(MISC_RXDV_GATE | MISC_L2L3)
}

/// Delai minimal entre deux reprises, en nanosecondes.
///
/// Une reprise coute des trames. En enchainer sans laisser au moteur le temps
/// de montrer qu'il est reparti transformerait une anomalie en boucle.
pub const REPOS_ENTRE_REPRISES_NS: u64 = 2_000_000_000;

/// La reception est-elle arretee ?
///
/// Les quatre conditions de l'enonce, et il faut les quatre : lien haut,
/// emission vivante, reception muette depuis trop longtemps, et une reprise
/// qui n'est pas deja toute recente.
pub fn reception_arretee(sante: &Sante, maintenant_ns: u64) -> bool {
    if !sante.lien {
        return false;
    }
    if sante.tx_dernier_ns == 0 {
        return false;
    }
    if sante.reprise_derniere_ns != 0
        && maintenant_ns.saturating_sub(sante.reprise_derniere_ns) < REPOS_ENTRE_REPRISES_NS
    {
        return false;
    }
    let reference = if sante.rx_dernier_ns == 0 {
        // Jamais rien recu : le silence se compte depuis l'instant ou nous
        // avons commence a parler. Voir `Sante::tx_premier_ns`.
        if sante.tx_premier_ns == 0 {
            return false;
        }
        sante.tx_premier_ns
    } else {
        sante.rx_dernier_ns
    };
    // NOUS ATTENDONS UNE REPONSE : nous avons parle APRES avoir entendu.
    //
    // C'est cela qui distingue une panne d'un reseau au repos, et non la
    // fraicheur de l'emission au moment precis ou l'on pose la question.
    if sante.tx_dernier_ns < reference {
        return false;
    }
    // ... ET L'ATTENTE N'EST PAS PERIMEE.
    //
    // # Pourquoi cette borne ne vaut pas SILENCE_RX_NS
    //
    // Elle les valait. Le releve physique du 18 septembre montre ce que cela
    // coutait : trois cent quarante secondes de reception morte, `isr_rx_ok`
    // qui continue de monter, et PAS UNE reprise -- la reparation n'a meme
    // jamais ete demandee.
    //
    // Le veilleur est un seul fil :
    //
    //     boucle { dormir(1 s) ; verifie_la_reception() ; ... ;
    //              dhcp::negocie_avant(4 000 ms) }
    //
    // `negocie_avant` emet, puis attend l'offre quatre secondes. Quand il
    // rend la main, la derniere emission a deja quatre secondes ; la boucle
    // dort une seconde de plus. Le chien de garde etait donc TOUJOURS
    // interroge au moins cinq secondes apres l'emission, alors qu'il exigeait
    // qu'elle ait moins de trois secondes.
    //
    // Le fil qui pouvait declencher la reprise etait precisement celui qui
    // restait bloque pendant la seule fenetre ou il en avait le droit. Aucun
    // reglage de seuil ne rattrape cela : un predicat ne doit pas dependre de
    // l'instant ou on l'interroge.
    //
    // Trente secondes laissent au veilleur vingt-cinq occasions de regarder
    // entre deux reemissions DHCP, et gardent la contrepartie : passe ce
    // delai, plus personne n'attend de reponse et une machine tranquille
    // cesse d'accuser sa carte.
    if maintenant_ns.saturating_sub(sante.tx_dernier_ns) > ATTENTE_REPONSE_NS {
        return false;
    }
    maintenant_ns.saturating_sub(reference) >= SILENCE_RX_NS
}

/// Les degres de la reprise, du moins invasif au plus invasif.
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Degre {
    /// Drainer l'anneau et acquitter les statuts. Ne perd rien.
    Draine = 0,
    /// Rendre au materiel les descripteurs que le processeur retient.
    Rearme = 1,
    /// Relancer le moteur de reception seul.
    RelanceRx = 2,
    /// Reconstruire l'anneau : son invariant est casse.
    ReconstruitAnneau = 3,
    /// Reinitialiser la carte. Coupe le lien, et c'est pourquoi il est dernier.
    ReinitialiseCarte = 4,
}

/// Le degre qui repond a cet etat, et lui seul.
///
/// # Ne pas couper le lien pour une anomalie de reception
///
/// Une reinitialisation complete rend l'interface muette une seconde, fait
/// retomber le lien, et oblige a refaire DHCP. Ce prix ne se paie que si
/// l'anneau lui-meme est incoherent -- c'est-a-dire si le materiel et nous ne
/// parlons plus du meme objet.
pub fn degre(
    invariant: Option<&'static str>,
    recensement: Recensement,
    chip_cmd: u8,
    reprises_infructueuses: u32,
) -> Degre {
    if invariant.is_some() {
        return Degre::ReconstruitAnneau;
    }
    // Deux reprises qui n'ont rien change : le probleme n'est pas dans
    // l'anneau, il est dans la carte.
    if reprises_infructueuses >= 3 {
        return Degre::ReinitialiseCarte;
    }
    if reprises_infructueuses >= 2 {
        return Degre::ReconstruitAnneau;
    }
    if moteur_rx_a_relancer(chip_cmd) {
        return Degre::RelanceRx;
    }
    if recensement.processeur != 0 {
        return Degre::Rearme;
    }
    Degre::Draine
}
