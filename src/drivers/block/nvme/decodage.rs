// Le decodage NVM Express : tout ce qui est de l'arithmetique, et rien d'autre.
//
// Ce fichier ne touche aucun registre. Il est `include!` par le pilote et
// `#[path]`-inclus par `tools/platform/test_nvme_decodage.rs`, pour la meme
// raison que `pci/decodage.rs` : les cas qui font echouer un pilote NVMe ne se
// produisent pas a la demande.
//
// # Ce que le materiel ne pardonne pas
//
// Trois calculs de cette famille sont faux dans presque toute premiere
// version d'un pilote NVMe, et aucun des trois ne se voit sur une lecture
// d'un secteur :
//
//   * **La foulee des sonnettes.** Le registre de sonnette de la file `q` ne
//     se trouve pas a `0x1000 + q * 8`. Il se trouve a `0x1000 + q * (4 <<
//     DSTRD)`, et `DSTRD` vaut zero sur la quasi-totalite du materiel --
//     QEMU compris. Un pilote qui code `8` en dur marche partout ou on le
//     teste et sonne dans le vide sur le premier controleur qui annonce une
//     foulee differente.
//
//   * **Le nombre de blocs.** Le champ `NLB` d'une lecture est DECALE DE UN :
//     zero veut dire « un bloc ». Un pilote qui y ecrit le compte reel lit un
//     bloc de trop a chaque requete. Sur une lecture, ce bloc de trop est
//     jete ; sur une ECRITURE, il ecrase le bloc suivant, et le systeme de
//     fichiers se corrompt lentement, loin de la cause.
//
//   * **La liste PRP.** Une requete qui ne tient pas dans deux pages ne met
//     pas la troisieme page dans `PRP2` : elle y met l'adresse d'une LISTE.
//     La frontiere est a deux pages exactement, et un transfert de 8 Kio
//     depuis un tampon aligne tient dans deux pages tandis que le meme
//     transfert depuis un tampon decale n'y tient pas. C'est le decalage,
//     pas la taille, qui decide.
//
// # La taille de bloc n'est pas 512
//
// Elle vaut 512 sur beaucoup de disques et 4096 sur beaucoup d'autres, et le
// disque le dit dans `Identify Namespace` : `FLBAS` designe un format parmi
// seize, et ce format porte `LBADS`, un LOGARITHME. Un pilote qui suppose 512
// sur un disque formate en 4096 lit le bon nombre d'octets a la mauvaise
// adresse -- huit fois trop loin -- et ne s'en apercoit jamais tout seul.

/// Taille d'une entree de file de soumission, en octets.
pub const TAILLE_SQE: usize = 64;
/// Taille d'une entree de file d'achevement, en octets.
pub const TAILLE_CQE: usize = 16;

/// Decalages des registres du controleur, dans le BAR0.
pub mod registre {
    /// Capacites (64 bits).
    pub const CAP: usize = 0x00;
    /// Version (32 bits).
    pub const VS: usize = 0x08;
    /// Masque d'interruptions -- mise a un.
    pub const INTMS: usize = 0x0C;
    /// Masque d'interruptions -- mise a zero.
    pub const INTMC: usize = 0x10;
    /// Configuration du controleur (32 bits).
    pub const CC: usize = 0x14;
    /// Etat du controleur (32 bits).
    pub const CSTS: usize = 0x1C;
    /// Attributs des files admin (32 bits).
    pub const AQA: usize = 0x24;
    /// Base de la file de soumission admin (64 bits).
    pub const ASQ: usize = 0x28;
    /// Base de la file d'achevement admin (64 bits).
    pub const ACQ: usize = 0x30;
    /// Premiere sonnette.
    pub const SONNETTES: usize = 0x1000;
}

/// Codes d'operation admin.
pub mod admin {
    pub const SUPPRIME_SQ: u8 = 0x00;
    pub const CREE_SQ: u8 = 0x01;
    pub const SUPPRIME_CQ: u8 = 0x04;
    pub const CREE_CQ: u8 = 0x05;
    pub const IDENTIFIE: u8 = 0x06;
    pub const ABANDONNE: u8 = 0x08;
    pub const POSE_ATTRIBUT: u8 = 0x09;
    pub const LIT_ATTRIBUT: u8 = 0x0A;
}

/// Codes d'operation d'entree-sortie.
pub mod es {
    pub const VIDANGE: u8 = 0x00;
    pub const ECRITURE: u8 = 0x01;
    pub const LECTURE: u8 = 0x02;
}

/// Valeurs de `CNS` pour `Identify`.
pub mod cns {
    /// Le namespace designe par `NSID`.
    pub const NAMESPACE: u32 = 0x00;
    /// Le controleur.
    pub const CONTROLEUR: u32 = 0x01;
    /// La liste des namespaces actifs.
    pub const LISTE_NAMESPACES: u32 = 0x02;
}

/// Ce que `CAP` annonce.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacites {
    /// Entrees maximales par file, DEJA remise a l'endroit (le champ est
    /// decale de un dans le registre).
    pub entrees_max: u32,
    /// Foulee des sonnettes, en octets.
    pub foulee_sonnette: usize,
    /// Delai maximal que le controleur peut prendre pour devenir pret, en ms.
    pub delai_pret_ms: u32,
    /// Log2 de la taille de page minimale, moins 12.
    pub mps_min: u8,
    /// Log2 de la taille de page maximale, moins 12.
    pub mps_max: u8,
    /// Le controleur accepte-t-il le jeu de commandes NVM ?
    pub jeu_nvm: bool,
    /// Les files doivent-elles etre physiquement contigues ?
    pub files_contigues: bool,
}

/// Decode le registre `CAP`.
///
/// `MQES` est un champ DECALE DE UN, comme presque tous les comptes de la
/// specification : la valeur `0x1F` veut dire « trente-deux entrees ». Le
/// remettre a l'endroit ici, une fois, evite que chaque appelant se souvienne
/// de le faire -- et un appelant sur deux ne s'en souvient pas.
pub const fn decode_cap(cap: u64) -> Capacites {
    let mqes = (cap & 0xFFFF) as u32;
    let dstrd = ((cap >> 32) & 0xF) as u32;
    let to = ((cap >> 24) & 0xFF) as u32;
    Capacites {
        entrees_max: mqes + 1,
        foulee_sonnette: (4u32 << dstrd) as usize,
        // `TO` compte des unites de 500 ms. Un controleur qui annonce zero
        // n'est pas un controleur instantane : c'est un controleur qui n'a
        // rien annonce, et l'attendre zero milliseconde le declarerait mort
        // avant qu'il ait eu le droit de repondre.
        delai_pret_ms: if to == 0 { 500 } else { to * 500 },
        mps_min: ((cap >> 48) & 0xF) as u8,
        mps_max: ((cap >> 52) & 0xF) as u8,
        jeu_nvm: (cap >> 37) & 1 != 0,
        files_contigues: (cap >> 16) & 1 != 0,
    }
}

/// Le decalage, dans le BAR0, de la sonnette d'une file.
///
/// Les sonnettes alternent : soumission de la file 0, achevement de la file 0,
/// soumission de la file 1... La file 0 est la file ADMIN.
pub const fn decalage_sonnette(file: u16, achevement: bool, foulee: usize) -> usize {
    let index = (file as usize) * 2 + if achevement { 1 } else { 0 };
    registre::SONNETTES + index * foulee
}

/// La valeur a ecrire dans `AQA` pour des files admin de ces tailles.
///
/// Les deux champs sont decales de un.
pub const fn valeur_aqa(entrees_sq: u32, entrees_cq: u32) -> u32 {
    let sq = (entrees_sq - 1) & 0x0FFF;
    let cq = (entrees_cq - 1) & 0x0FFF;
    (cq << 16) | sq
}

/// La valeur a ecrire dans `CC` pour demarrer le controleur.
///
/// `IOSQES`/`IOCQES` sont des LOGARITHMES de la taille d'une entree : 6 pour
/// 64 octets, 4 pour 16. Les ecrire en octets programme des files de six et
/// quatre octets, et le controleur lit alors ses commandes n'importe ou.
pub const fn valeur_cc_demarrage(mps: u8) -> u32 {
    let iosqes: u32 = 6;
    let iocqes: u32 = 4;
    (iocqes << 20) | (iosqes << 16) | ((mps as u32 & 0xF) << 7) | (0 << 4) | 1
}

/// `CSTS.RDY`.
pub const fn pret(csts: u32) -> bool {
    csts & 1 != 0
}

/// `CSTS.CFS` -- panne fatale du controleur.
///
/// La distinguer de « pas encore pret » est ce qui evite d'attendre le delai
/// entier un controleur qui a deja abandonne.
pub const fn panne_fatale(csts: u32) -> bool {
    csts & 0x2 != 0
}

// ---------------------------------------------------------------------------
// Les commandes
// ---------------------------------------------------------------------------

/// Une entree de file de soumission, vue comme seize mots de 32 bits.
///
/// La representation en mots plutot qu'en `#[repr(C)]` est deliberee : elle
/// rend la commande comparable octet par octet dans un test hote, ce qu'une
/// structure a champs nommes ne permet pas sans transmutation.
pub type Sqe = [u32; 16];

/// La commande vide.
pub const fn sqe_vide() -> Sqe {
    [0u32; 16]
}

/// Pose l'en-tete commun d'une commande.
pub const fn sqe_entete(opcode: u8, identifiant: u16, nsid: u32) -> Sqe {
    let mut sqe = sqe_vide();
    sqe[0] = (opcode as u32) | ((identifiant as u32) << 16);
    sqe[1] = nsid;
    sqe
}

/// Pose `PRP1` et `PRP2`.
pub const fn sqe_prp(mut sqe: Sqe, prp1: u64, prp2: u64) -> Sqe {
    sqe[6] = prp1 as u32;
    sqe[7] = (prp1 >> 32) as u32;
    sqe[8] = prp2 as u32;
    sqe[9] = (prp2 >> 32) as u32;
    sqe
}

/// Une lecture ou une ecriture.
///
/// `blocs` est le compte REEL. La conversion vers le champ decale de un se
/// fait ici et nulle part ailleurs.
pub const fn commande_transfert(
    ecriture: bool,
    identifiant: u16,
    nsid: u32,
    lba: u64,
    blocs: u32,
    prp1: u64,
    prp2: u64,
) -> Sqe {
    let opcode = if ecriture { es::ECRITURE } else { es::LECTURE };
    let mut sqe = sqe_prp(sqe_entete(opcode, identifiant, nsid), prp1, prp2);
    sqe[10] = lba as u32;
    sqe[11] = (lba >> 32) as u32;
    // NLB est decale de un : zero veut dire un bloc.
    sqe[12] = blocs.saturating_sub(1) & 0xFFFF;
    sqe
}

/// Une vidange de cache.
pub const fn commande_vidange(identifiant: u16, nsid: u32) -> Sqe {
    sqe_entete(es::VIDANGE, identifiant, nsid)
}

/// Une commande `Identify`.
pub const fn commande_identifie(identifiant: u16, nsid: u32, cns: u32, prp1: u64) -> Sqe {
    let mut sqe = sqe_prp(sqe_entete(admin::IDENTIFIE, identifiant, nsid), prp1, 0);
    sqe[10] = cns;
    sqe
}

/// La creation d'une file d'achevement d'entree-sortie.
///
/// `PC` (bit 0 de dw11) dit « physiquement contigue ». `IEN` (bit 1) active
/// l'interruption ; ce pilote scrute, et le laisse a zero -- activer une
/// interruption qu'aucun vecteur ne recoit ferait monter un IRQ non gere.
pub const fn commande_cree_cq(identifiant: u16, file: u16, entrees: u32, base: u64) -> Sqe {
    let mut sqe = sqe_prp(sqe_entete(admin::CREE_CQ, identifiant, 0), base, 0);
    sqe[10] = (file as u32) | (((entrees - 1) & 0xFFFF) << 16);
    sqe[11] = 1;
    sqe
}

/// La creation d'une file de soumission d'entree-sortie.
///
/// dw11 porte `PC` en bit 0, la priorite en bits 2:1, et l'identifiant de la
/// file d'ACHEVEMENT associee en bits 31:16. Oublier ce dernier champ cree une
/// file dont les achevements partent vers la file admin, ou personne ne les
/// attend.
pub const fn commande_cree_sq(
    identifiant: u16,
    file: u16,
    entrees: u32,
    base: u64,
    file_achevement: u16,
) -> Sqe {
    let mut sqe = sqe_prp(sqe_entete(admin::CREE_SQ, identifiant, 0), base, 0);
    sqe[10] = (file as u32) | (((entrees - 1) & 0xFFFF) << 16);
    sqe[11] = 1 | ((file_achevement as u32) << 16);
    sqe
}

/// `Set Features` / nombre de files demandees.
///
/// Les deux champs sont decales de un, et la reponse du controleur dit
/// combien il en ACCORDE -- qui peut etre moins.
pub const fn commande_nombre_de_files(identifiant: u16, files: u16) -> Sqe {
    let mut sqe = sqe_entete(admin::POSE_ATTRIBUT, identifiant, 0);
    sqe[10] = 0x07;
    let n = (files.saturating_sub(1)) as u32;
    sqe[11] = n | (n << 16);
    sqe
}

// ---------------------------------------------------------------------------
// Les achevements
// ---------------------------------------------------------------------------

/// Ce qu'une entree d'achevement dit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntreeAchevement {
    pub identifiant: u16,
    /// Bit de phase. Il ALTERNE a chaque tour de file ; c'est lui, et non un
    /// contenu remis a zero, qui distingue une entree neuve d'une ancienne.
    pub phase: bool,
    /// Type de statut.
    pub type_statut: u8,
    /// Code de statut.
    pub code_statut: u8,
    /// Tete de la file de soumission, telle que le controleur la voit.
    pub tete_sq: u16,
    pub file_sq: u16,
    pub mot0: u32,
}

impl EntreeAchevement {
    /// La commande a-t-elle reussi ?
    pub const fn reussi(&self) -> bool {
        self.type_statut == 0 && self.code_statut == 0
    }
}

/// Decode une entree d'achevement.
pub const fn decode_achevement(cqe: [u32; 4]) -> EntreeAchevement {
    let dw3 = cqe[3];
    EntreeAchevement {
        identifiant: (dw3 & 0xFFFF) as u16,
        phase: (dw3 >> 16) & 1 != 0,
        code_statut: ((dw3 >> 17) & 0xFF) as u8,
        type_statut: ((dw3 >> 25) & 0x7) as u8,
        tete_sq: (cqe[2] & 0xFFFF) as u16,
        file_sq: ((cqe[2] >> 16) & 0xFFFF) as u16,
        mot0: cqe[0],
    }
}

// ---------------------------------------------------------------------------
// Les listes PRP
// ---------------------------------------------------------------------------

/// Ce que `PRP1` et `PRP2` doivent porter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prp {
    /// Tout tient dans la premiere page : `PRP2` est inutilise.
    UnePage { prp1: u64 },
    /// Deux pages : `PRP2` porte directement la seconde.
    DeuxPages { prp1: u64, prp2: u64 },
    /// Au-dela : `PRP2` porte l'adresse d'une liste de `entrees` adresses,
    /// dont la premiere est la DEUXIEME page du transfert.
    Liste { prp1: u64, entrees: usize },
}

/// Ce que `PRP1`/`PRP2` doivent porter pour ce transfert.
///
/// `physique` est l'adresse du premier octet -- pas de la page qui le
/// contient. Le decalage dans la page est ce qui decide de tout : un transfert
/// de deux pages depuis une adresse alignee occupe deux pages, le meme
/// transfert depuis une adresse decalee d'un octet en occupe trois, et bascule
/// donc dans le cas « liste ».
pub const fn plan_prp(physique: u64, octets: usize, page: usize) -> Prp {
    let decalage = (physique as usize) & (page - 1);
    let premiere = octets_dans_la_premiere_page(decalage, octets, page);
    if octets <= premiere {
        return Prp::UnePage { prp1: physique };
    }
    let base_seconde = (physique - decalage as u64) + page as u64;
    let reste = octets - premiere;
    if reste <= page {
        return Prp::DeuxPages { prp1: physique, prp2: base_seconde };
    }
    // Le nombre de pages APRES la premiere, arrondi vers le haut.
    let entrees = (reste + page - 1) / page;
    Prp::Liste { prp1: physique, entrees }
}

/// Combien d'octets du transfert tiennent dans la page du premier octet.
pub const fn octets_dans_la_premiere_page(decalage: usize, octets: usize, page: usize) -> usize {
    let disponible = page - decalage;
    if octets < disponible { octets } else { disponible }
}

/// L'adresse de la `n`-ieme page d'un transfert, `n` comptant a partir de la
/// SECONDE page -- c'est-a-dire la `n`-ieme entree d'une liste PRP.
pub const fn page_de_la_liste(physique: u64, n: usize, page: usize) -> u64 {
    let base = physique & !((page as u64) - 1);
    base + ((n + 1) as u64) * page as u64
}

// ---------------------------------------------------------------------------
// Identify Namespace
// ---------------------------------------------------------------------------

/// Le format de bloc en service sur un namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatBloc {
    pub taille_bloc: usize,
    pub metadonnees: usize,
}

/// Lit un mot de 64 bits en petit-boutiste dans un tampon.
pub fn lit_u64(tampon: &[u8], decalage: usize) -> u64 {
    let mut valeur = 0u64;
    let mut i = 0;
    while i < 8 {
        match tampon.get(decalage + i) {
            Some(octet) => valeur |= (*octet as u64) << (8 * i),
            None => return 0,
        }
        i += 1;
    }
    valeur
}

/// Lit un mot de 32 bits en petit-boutiste dans un tampon.
pub fn lit_u32(tampon: &[u8], decalage: usize) -> u32 {
    let mut valeur = 0u32;
    let mut i = 0;
    while i < 4 {
        match tampon.get(decalage + i) {
            Some(octet) => valeur |= (*octet as u32) << (8 * i),
            None => return 0,
        }
        i += 1;
    }
    valeur
}

/// Le format de bloc EN SERVICE, extrait d'un `Identify Namespace`.
///
/// `FLBAS` bits 3:0 designe l'un des seize formats decrits a partir de l'octet
/// 128. Le format porte `LBADS`, un LOGARITHME en base deux de la taille du
/// bloc : la valeur `9` veut dire 512, la valeur `12` veut dire 4096.
///
/// Rend `None` quand le format designe est inutilisable -- `LBADS` inferieur a
/// neuf n'est pas un bloc, c'est un champ non initialise. Supposer 512 dans ce
/// cas serait la pire des sorties : le pilote calculerait des adresses fausses
/// sans jamais se signaler.
pub fn format_bloc(identify: &[u8]) -> Option<FormatBloc> {
    if identify.len() < 132 {
        return None;
    }
    let flbas = identify[26];
    let index = (flbas & 0x0F) as usize;
    let nlbaf = identify[25] as usize;
    if index > nlbaf.min(15) {
        return None;
    }
    let brut = lit_u32(identify, 128 + index * 4);
    let lbads = ((brut >> 16) & 0xFF) as u32;
    if !(9..=24).contains(&lbads) {
        return None;
    }
    Some(FormatBloc {
        taille_bloc: 1usize << lbads,
        metadonnees: (brut & 0xFFFF) as usize,
    })
}

/// La taille du namespace en blocs, extraite d'un `Identify Namespace`.
pub fn blocs_du_namespace(identify: &[u8]) -> u64 {
    lit_u64(identify, 0)
}

/// Le nombre maximal d'octets qu'une commande de transfert peut porter.
///
/// `MDTS` d'`Identify Controller` (octet 77) est un LOGARITHME du nombre de
/// pages minimales. Zero veut dire « pas de limite annoncee » -- et non « zero
/// octet », qui rendrait tout transfert impossible.
pub fn transfert_max_octets(identify_controleur: &[u8], page_min: usize) -> Option<usize> {
    let mdts = *identify_controleur.get(77)?;
    if mdts == 0 || mdts >= 32 {
        return None;
    }
    Some(page_min << mdts)
}

/// Les identifiants de namespace actifs listes par `Identify` CNS=2.
///
/// La liste est une suite de mots de 32 bits terminee par un zero. Elle n'est
/// PAS forcement `1, 2, 3...` : un disque peut n'exposer que le namespace 7.
/// Supposer le namespace 1 marche sur presque tout le materiel grand public et
/// echoue silencieusement sur le reste.
pub fn namespaces_actifs(liste: &[u8], sortie: &mut [u32]) -> usize {
    let mut n = 0;
    let mut decalage = 0;
    while decalage + 4 <= liste.len() && n < sortie.len() {
        let nsid = lit_u32(liste, decalage);
        if nsid == 0 {
            break;
        }
        sortie[n] = nsid;
        n += 1;
        decalage += 4;
    }
    n
}
