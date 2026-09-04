// Le decodage de l'espace de configuration PCI, sans acces materiel.
//
// CE QUE L'ENUMERATION NE VOYAIT PAS
// ==================================
//
// Elle balayait le BUS 0, et rien d'autre. C'est suffisant sur i440fx, ou tout
// est branche sur le bus unique -- et c'est exactement faux sur Q35, la
// plateforme de reference moderne : les peripheriques y sont derriere des
// PONTS RACINE PCIe, donc sur des bus 1, 2, 3... Un controleur NVMe attache a
// un port racine est INVISIBLE a un balayage du bus 0, et le systeme conclut
// « pas de disque » alors que le disque est la.
//
// Elle ne lisait pas non plus la LISTE DE CAPACITES. MSI et MSI-X y vivent, et
// sans elles un controleur moderne ne peut delivrer ses interruptions que par
// la ligne heritee -- partagee, lente, et absente de certaines topologies
// PCIe. C'est la premiere brique que NVMe demande.
//
// Et elle lisait les BAR par mots de 32 bits. Le BAR0 d'un NVMe est un BAR
// MEMOIRE 64 BITS : lu sur 32 bits, il donne la moitie basse d'une adresse, ce
// qui est pire qu'une erreur -- c'est une adresse plausible qui pointe ailleurs.
//
// POURQUOI CE MODULE EST PUR
// ==========================
//
// Il ne touche ni `0xCF8` ni `0xCFC` : il recoit un LECTEUR. Le noyau lui donne
// les ports ; un test hote lui donne un espace de configuration fabrique, avec
// une chaine de capacites, un BAR 64 bits, et une boucle -- un materiel abime
// ou hostile peut chainer une capacite sur elle-meme, et un parcours naif y
// tourne pour toujours.

/// Decalage de la liste de capacites dans l'en-tete de type 0 et 1.
pub const OFFSET_CAPACITES: u8 = 0x34;
/// Bit 4 du registre d'etat : la liste de capacites existe.
pub const STATUT_CAPACITES: u16 = 1 << 4;

/// Identifiants de capacite qui nous interessent.
pub const CAP_MSI: u8 = 0x05;
pub const CAP_PCIE: u8 = 0x10;
pub const CAP_MSIX: u8 = 0x11;

/// Capacites parcourues avant d'abandonner.
///
/// La liste est chainee dans un espace de 256 octets : au-dela de sa taille,
/// c'est une BOUCLE. Un materiel abime -- ou un peripherique hostile branche a
/// chaud -- suffit a figer un parcours naif.
pub const CAPACITES_MAX: usize = 48;

/// Classe et sous-classe d'un controleur NVMe.
pub const CLASSE_STOCKAGE: u8 = 0x01;
pub const SOUS_CLASSE_NVME: u8 = 0x08;
/// Interface de programmation d'un NVM Express.
pub const PROGIF_NVME: u8 = 0x02;

/// Classe et sous-classe d'un pont PCI-vers-PCI.
pub const CLASSE_PONT: u8 = 0x06;
pub const SOUS_CLASSE_PONT_PCI: u8 = 0x04;

/// Une capacite trouvee dans la liste chainee.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacite {
    pub identifiant: u8,
    pub decalage: u8,
}

/// Ce qu'un BAR decrit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bar {
    /// Espace d'entree-sortie.
    Port(u32),
    /// Espace memoire 32 bits.
    Memoire32 { adresse: u32, prefetch: bool },
    /// Espace memoire 64 bits : il occupe DEUX emplacements de BAR.
    Memoire64 { adresse: u64, prefetch: bool },
    /// Emplacement vide, ou moitie haute d'un BAR 64 bits.
    Absent,
}

impl Bar {
    /// L'adresse physique, quelle que soit la forme.
    pub const fn adresse(self) -> u64 {
        match self {
            Bar::Port(port) => port as u64,
            Bar::Memoire32 { adresse, .. } => adresse as u64,
            Bar::Memoire64 { adresse, .. } => adresse,
            Bar::Absent => 0,
        }
    }

    /// Ce BAR consomme-t-il l'emplacement suivant ?
    pub const fn double(self) -> bool {
        matches!(self, Bar::Memoire64 { .. })
    }
}

/// Decode un BAR a partir de son mot bas et, si besoin, de son mot haut.
///
/// `haut` n'est lu que pour un BAR memoire 64 bits ; le passer autrement est
/// sans effet. Rendre `Absent` pour un mot nul est ce qui evite de fabriquer
/// une adresse zero « valide », qui se manifesterait bien plus tard sous la
/// forme d'une ecriture materielle a l'adresse physique zero.
pub const fn decode_bar(bas: u32, haut: u32) -> Bar {
    if bas == 0 {
        return Bar::Absent;
    }
    if bas & 1 != 0 {
        return Bar::Port(bas & !0x3);
    }
    let prefetch = bas & 0x8 != 0;
    let type_memoire = (bas >> 1) & 0x3;
    let base = (bas & !0xF) as u64;
    if type_memoire == 0x2 {
        Bar::Memoire64 { adresse: base | ((haut as u64) << 32), prefetch }
    } else {
        Bar::Memoire32 { adresse: bas & !0xF, prefetch }
    }
}

/// Parcourt la liste de capacites.
///
/// `lit32` lit un mot de l'espace de configuration de CE peripherique.
/// `statut_commande` est le mot 0x04 (etat en haut, commande en bas).
///
/// Le parcours est BORNE et refuse les decalages qui reviennent en arriere : la
/// liste est chainee dans 256 octets, et un chainage qui boucle -- materiel
/// abime, peripherique hostile -- figerait un parcours naif pour toujours.
pub fn capacites<F: FnMut(u8) -> u32>(
    statut_commande: u32,
    mut lit32: F,
    sortie: &mut [Capacite],
) -> usize {
    let statut = (statut_commande >> 16) as u16;
    if statut & STATUT_CAPACITES == 0 {
        return 0;
    }
    let mut decalage = (lit32(OFFSET_CAPACITES) & 0xFC) as u8;
    let mut trouvees = 0usize;
    let mut vus = [0u8; CAPACITES_MAX];
    let mut nombre_vus = 0usize;

    while decalage >= 0x40 && trouvees < sortie.len() && nombre_vus < CAPACITES_MAX {
        // Deja visite : c'est une boucle, et il n'y a rien de plus a apprendre.
        let mut boucle = false;
        for index in 0..nombre_vus {
            if vus[index] == decalage {
                boucle = true;
                break;
            }
        }
        if boucle {
            break;
        }
        vus[nombre_vus] = decalage;
        nombre_vus += 1;

        let mot = lit32(decalage);
        let identifiant = (mot & 0xFF) as u8;
        let suivant = ((mot >> 8) & 0xFC) as u8;
        sortie[trouvees] = Capacite { identifiant, decalage };
        trouvees += 1;
        decalage = suivant;
    }
    trouvees
}

/// Cherche une capacite par identifiant.
pub fn trouve_capacite(capacites: &[Capacite], identifiant: u8) -> Option<Capacite> {
    let mut index = 0;
    while index < capacites.len() {
        if capacites[index].identifiant == identifiant {
            return Some(capacites[index]);
        }
        index += 1;
    }
    None
}

/// Le nombre de vecteurs qu'un MSI-X annonce.
///
/// Le champ « Table Size » est un nombre de vecteurs MOINS UN : le lire tel
/// quel donne toujours un vecteur de trop peu, et la derniere file d'un NVMe
/// n'aurait jamais d'interruption.
pub const fn vecteurs_msix(controle: u16) -> u16 {
    (controle & 0x7FF) + 1
}

/// Le nombre de vecteurs qu'un MSI peut prendre.
///
/// Le champ est un LOGARITHME : 0 signifie un vecteur, 1 en signifie deux,
/// jusqu'a 5 pour trente-deux. Le lire comme un compte donnerait cinq vecteurs
/// pour un peripherique qui en demande trente-deux.
pub const fn vecteurs_msi(controle: u16) -> u16 {
    let logarithme = (controle >> 1) & 0x7;
    if logarithme > 5 { 32 } else { 1u16 << logarithme }
}

/// Ce peripherique est-il un pont PCI-vers-PCI ?
///
/// C'est la question que l'enumeration ne posait pas, et c'est pour cela qu'un
/// NVMe derriere un port racine Q35 etait invisible.
pub const fn est_pont(classe: u8, sous_classe: u8, type_entete: u8) -> bool {
    classe == CLASSE_PONT
        && sous_classe == SOUS_CLASSE_PONT_PCI
        && (type_entete & 0x7F) == 0x01
}

/// Le bus secondaire d'un pont, lu dans son mot 0x18.
pub const fn bus_secondaire(mot_18: u32) -> u8 {
    ((mot_18 >> 8) & 0xFF) as u8
}

/// Le bus subordonne : le plus grand numero de bus derriere ce pont.
pub const fn bus_subordonne(mot_18: u32) -> u8 {
    ((mot_18 >> 16) & 0xFF) as u8
}

/// Ce peripherique est-il un controleur NVM Express ?
pub const fn est_nvme(classe: u8, sous_classe: u8, prog_if: u8) -> bool {
    classe == CLASSE_STOCKAGE
        && sous_classe == SOUS_CLASSE_NVME
        && prog_if == PROGIF_NVME
}

/// Ce peripherique a-t-il plusieurs fonctions ?
///
/// Sans ce bit, l'enumeration balaie les huit fonctions de chaque emplacement,
/// ce qui marche et fait huit fois trop d'acces de configuration. Avec, elle
/// n'insiste que la ou il y a quelque chose.
pub const fn multifonction(type_entete: u8) -> bool {
    type_entete & 0x80 != 0
}
