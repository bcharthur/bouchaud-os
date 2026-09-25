//! BRDP/1 -- Bouchaud Remote Debug Protocol. Le protocole, et rien que lui.
//!
//! # Ce que ce n'est PAS
//!
//! Ce n'est pas SSH, et cela ne pretend pas l'etre. SSH veut un echange de
//! cles asymetrique, une negociation d'algorithmes, des canaux multiplexes et
//! un shell : quatre chantiers dont aucun ne sert l'enquete en cours. BRDP a
//! une liste blanche de commandes de diagnostic, et pas de shell du tout.
//!
//! # Le format
//!
//! Une ligne JSON par message, dans les deux sens, terminee par `\n`. Pas de
//! longueur prefixee, pas de trame binaire : un releve doit pouvoir se lire
//! avec `nc` quand le client Python lui-meme est en cause.
//!
//! ```text
//! S> {"brdp":1,"nonce":"a1b2...","auth":"hmac-sha256"}
//! C> {"cmd":"hello","mac":"3f4e..."}
//! S> {"ok":true,"lab":1,"events":41}
//! C> {"cmd":"rtl8168 desc","n":63}
//! S> {"ok":true,...}
//! ```
//!
//! # L'authentification
//!
//! Le serveur ouvre avec un nonce ; le client prouve qu'il connait le jeton en
//! rendant `HMAC-SHA256(jeton, nonce)`. Le jeton ne traverse JAMAIS le reseau,
//! et un nonce par connexion interdit le rejeu.
//!
//! La comparaison est a temps constant. Sans cela, un attaquant sur le
//! segment local retrouverait le HMAC octet par octet en mesurant le temps de
//! reponse -- trente-deux fois deux cent cinquante-six essais au lieu de deux
//! puissance deux cent cinquante-six.
//!
//! # Pourquoi ce module est pur
//!
//! Pas de chaussette, pas d'etat global, pas d'horloge. Des octets entrent,
//! des decisions sortent. La fragmentation TCP, les commandes multiples dans
//! un segment, la ligne trop longue et le HMAC faux se contredisent donc en
//! test hote, sans pile reseau -- et ce sont precisement les cas qu'un banc
//! avec pile reseau reproduit le plus mal.

use crate::net::security::tls::sha256;

/// Version du protocole. Elle est dans le premier message, et le client la lit.
pub const VERSION: u32 = 1;

/// Longueur maximale d'une ligne, en octets.
///
/// # Pourquoi une borne, et pourquoi celle-la
///
/// Un tampon de ligne qui grandit avec l'entree est une panne memoire qu'un
/// pair hostile -- ou simplement casse -- declenche en envoyant un flux sans
/// retour a la ligne. Cinq cent douze octets tiennent largement la plus longue
/// commande du protocole (un HMAC en hexadecimal, soixante-quatre caracteres)
/// avec de la marge, et la borne est fixe.
pub const LIGNE_MAX: usize = 512;

/// Longueur du nonce, en octets. Trente-deux : la taille du HMAC.
pub const NONCE_LEN: usize = 32;

/// Ce qu'une ligne mal formee vaut. Chiffre, pour tenir dans l'anneau LAB.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u16)]
pub enum Erreur {
    /// Ligne vide. Tolerée : un client qui envoie `\n\n` n'est pas hostile.
    Vide = 1,
    /// Ne ressemble pas a un objet JSON.
    PasUnObjet = 2,
    /// Pas de champ `cmd`.
    CommandeAbsente = 3,
    /// Le champ `cmd` ne figure pas dans la liste blanche.
    CommandeInconnue = 4,
    /// Un argument obligatoire manque.
    ArgumentManquant = 5,
    /// Un argument est hors bornes.
    ArgumentInvalide = 6,
    /// La ligne depassait `LIGNE_MAX`.
    TropLongue = 7,
    /// Commande envoyee avant `hello`, ou apres un `hello` refuse.
    NonAuthentifie = 8,
    /// Le HMAC ne correspond pas.
    AuthRefusee = 9,
    /// Trop de commandes en attente : celle-ci a ete refusee.
    ///
    /// Le serveur SERIALISE ses reponses et retient ce qui arrive pendant ce
    /// temps, dans une file bornee. Un client qui envoie sans jamais lire
    /// finit par la remplir. Le lui DIRE est la seule facon qu'il ait de
    /// distinguer « ma commande a ete refusee » de « la machine est figee ».
    FilePleine = 10,
}

/// LA LISTE BLANCHE. Il n'y a pas de shell, et il n'y en aura pas en V1.
///
/// Une commande arbitraire sur un canal d'enquete, c'est un acces root sur le
/// segment local au premier jeton qui fuit. Chacune de ces commandes LIT un
/// etat que la machine publie deja ; aucune n'ecrit, sauf
/// `BlackboxCheckpoint`, dont l'effet est precisement celui qu'on veut
/// pouvoir declencher a distance.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Commande {
    /// L'authentification. Porte le HMAC du nonce.
    Hello { mac: [u8; 32] },
    Status,
    AuditStatus,
    AuditRun,
    AuditLast,
    NetStatus,
    Rtl8168Status,
    Rtl8168Ring,
    /// Un descripteur precis. `n` est borne par le nombre de descripteurs.
    Rtl8168Desc { index: u16 },
    DhcpStatus,
    BlackboxStatus,
    BlackboxCheckpoint,
    /// Les `n` derniers evenements.
    EventsTail { combien: u32 },
    /// Le flux continu, a partir du curseur courant.
    EventsWatch,
    ServicesSnapshot,
    // BOUCHAUD_HOTFIX12_SERVICES_REMOTE_DETAIL_V1
    /// Page detaillee du registre central des services.
    ServicesPage { start: u16 },
    ProcessesSnapshot,
    MemorySnapshot,
    /// Lance la preuve Internet active asynchrone.
    InternetProofStart,
    /// Lit la progression de la preuve sans effet de bord.
    InternetProofStatus,
    // BOUCHAUD_HOTFIX11_SERIAL_BRDP
    /// Bornes du journal serie RAM.
    SerialStatus,
    /// Lecture bornee d'une tranche encore presente dans l'anneau serie.
    SerialRead { start: u64, combien: u16 },

    // BOUCHAUD_P0_REMOTE_CONTROL_V1
    SystemReboot,
    SystemShutdown,
    BrowserStart,
    BrowserStop,
    BrowserRestart,
    ProcessKill { pid: u32 },
    ProcessKillTree { pid: u32 },

    Quit,
}

impl Commande {
    /// Le code de la commande, pour l'anneau LAB.
    pub fn code(&self) -> u16 {
        match self {
            Commande::Hello { .. } => 1,
            Commande::Status => 2,
            Commande::AuditStatus => 3,
            Commande::AuditRun => 4,
            Commande::AuditLast => 5,
            Commande::NetStatus => 6,
            Commande::Rtl8168Status => 7,
            Commande::Rtl8168Ring => 8,
            Commande::Rtl8168Desc { .. } => 9,
            Commande::DhcpStatus => 10,
            Commande::BlackboxStatus => 11,
            Commande::BlackboxCheckpoint => 12,
            Commande::EventsTail { .. } => 13,
            Commande::EventsWatch => 14,
            Commande::ServicesSnapshot => 15,
            // BOUCHAUD_HOTFIX12_SERVICES_REMOTE_DETAIL_V1
            Commande::ServicesPage { .. } => 92,
            Commande::ProcessesSnapshot => 16,
            Commande::MemorySnapshot => 17,
            // BOUCHAUD_HOTFIX11_SERIAL_BRDP
            Commande::SerialStatus => 90,
            Commande::SerialRead { .. } => 91,
            Commande::Quit => 18,
            Commande::InternetProofStart => 19,
            Commande::InternetProofStatus => 20,
            // BOUCHAUD_P0_REMOTE_CONTROL_V1
            Commande::SystemReboot => 100,
            Commande::SystemShutdown => 101,
            Commande::BrowserStart => 110,
            Commande::BrowserStop => 111,
            Commande::BrowserRestart => 112,
            Commande::ProcessKill { .. } => 120,
            Commande::ProcessKillTree { .. } => 121,
        }
    }

    pub fn est_controle(&self) -> bool {
        matches!(self,
            Commande::SystemReboot | Commande::SystemShutdown |
            Commande::BrowserStart | Commande::BrowserStop | Commande::BrowserRestart |
            Commande::ProcessKill { .. } | Commande::ProcessKillTree { .. })
    }

    pub fn cible_controle(&self) -> u64 {
        match self {
            Commande::ProcessKill { pid } | Commande::ProcessKillTree { pid } => *pid as u64,
            _ => 0,
        }
    }

    /// Cette commande est-elle recevable avant l'authentification ?
    ///
    /// `hello` seule. Tout le reste attend, y compris `status` -- l'etat d'une
    /// machine est deja un renseignement.
    pub fn avant_auth(&self) -> bool {
        matches!(self, Commande::Hello { .. })
    }
}

/// Bornes du descripteur demandable. La carte en a soixante-quatre.
pub const DESCRIPTEURS_MAX: u16 = 64;
/// Plafond d'evenements rendus par `events tail`.
///
/// Un client qui demanderait dix mille evenements ferait ecrire la machine
/// pendant des secondes sur un canal que la panne rend deja fragile.
pub const EVENTS_TAIL_MAX: u32 = 1024;
// BOUCHAUD_HOTFIX11_SERIAL_BRDP
/// Une reponse fait 4096 octets. L'hexadecimal double la charge utile:
/// 1536 octets -> 3072 caracteres, avec une marge confortable pour le JSON.
pub const SERIAL_READ_MAX: u16 = 1536;

// BOUCHAUD_HOTFIX12_SERVICES_REMOTE_DETAIL_V1
/// Le registre central est borne a 128 entrees (`services::registre`).
pub const SERVICES_REMOTE_MAX: u16 = 128;


// ---------------------------------------------------------------------------
// LE DECOUPAGE EN LIGNES
// ---------------------------------------------------------------------------

/// Ce qu'un decoupage produit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Morceau<'a> {
    /// Une ligne complete, sans son terminateur.
    Ligne(&'a [u8]),
    /// Une ligne a depasse `LIGNE_MAX` et a ete abandonnee.
    ///
    /// Le flux N'EST PAS desynchronise : tout est jete jusqu'au prochain
    /// retour a la ligne. Fermer la connexion serait plus simple et
    /// nettement moins utile -- un client qui bafouille une fois doit
    /// pouvoir continuer.
    TropLongue,
}

/// Recompose des lignes a partir d'un flux TCP quelconque.
///
/// # Ce que TCP ne garantit pas
///
/// TCP est un flux d'octets : il ne promet ni qu'une commande arrive en un
/// seul segment, ni qu'un segment ne contient qu'une commande. Les deux cas
/// se produisent en vrai -- le premier sur un reseau charge, le second quand
/// le client envoie plusieurs commandes d'affilee -- et un serveur qui
/// suppose « un segment, une commande » marche au banc et casse sur le
/// terrain.
pub struct Decoupeur {
    tampon: [u8; LIGNE_MAX],
    n: usize,
    /// La ligne en cours a deja depasse la borne : on jette jusqu'au `\n`.
    en_purge: bool,
    lignes: u64,
    trop_longues: u64,
}

impl Default for Decoupeur {
    fn default() -> Self {
        Self::neuf()
    }
}

impl Decoupeur {
    pub const fn neuf() -> Self {
        Self {
            tampon: [0; LIGNE_MAX],
            n: 0,
            en_purge: false,
            lignes: 0,
            trop_longues: 0,
        }
    }

    pub fn lignes(&self) -> u64 {
        self.lignes
    }

    pub fn trop_longues(&self) -> u64 {
        self.trop_longues
    }

    /// Octets en attente d'un retour a la ligne.
    pub fn en_attente(&self) -> usize {
        self.n
    }

    /// Repart de zero. Pour une nouvelle connexion sur le meme decoupeur.
    pub fn reinitialise(&mut self) {
        self.n = 0;
        self.en_purge = false;
    }

    /// Avale des octets et rend chaque ligne complete au rappel.
    ///
    /// Le rappel peut etre appele zero, une ou plusieurs fois par appel.
    pub fn pousse(&mut self, octets: &[u8], mut rappel: impl FnMut(Morceau<'_>)) {
        for &octet in octets {
            if octet == b'\n' {
                if self.en_purge {
                    self.en_purge = false;
                    self.n = 0;
                    self.trop_longues += 1;
                    rappel(Morceau::TropLongue);
                    continue;
                }
                // `\r\n` autant que `\n` : un client ecrit en PowerShell ou en
                // Python sous Windows ne doit pas echouer sur un octet.
                let mut fin = self.n;
                if fin > 0 && self.tampon[fin - 1] == b'\r' {
                    fin -= 1;
                }
                self.lignes += 1;
                rappel(Morceau::Ligne(&self.tampon[..fin]));
                self.n = 0;
                continue;
            }
            if self.en_purge {
                continue;
            }
            if self.n == LIGNE_MAX {
                // LA BORNE EST ATTEINTE : on entre en purge, et on ne rendra
                // le verdict qu'au prochain `\n`. Le rendre tout de suite
                // ferait croire au client que sa ligne est finie.
                self.en_purge = true;
                continue;
            }
            self.tampon[self.n] = octet;
            self.n += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// UN ANALYSEUR JSON MINIMAL, ET DELIBEREMENT MINIMAL
// ---------------------------------------------------------------------------
//
// Pas d'arbre, pas d'allocation, pas de recursion : on cherche deux ou trois
// champs plats dans un objet qu'on a nous-memes specifie. Un analyseur
// general serait plus de code, plus de surface, et rien de plus pour ce
// protocole -- dont les messages entrants ont au plus trois champs.
//
// Ce qu'il REFUSE est aussi important que ce qu'il accepte : une cle
// cherchee ne doit pas se trouver dans une valeur (`{"cmd":"x","t":"cmd"}`).

fn est_blanc(o: u8) -> bool {
    matches!(o, b' ' | b'\t' | b'\r' | b'\n')
}

/// Cherche `"<cle>"` comme CLE, c'est-a-dire suivie d'un deux-points.
///
/// Rend la position du premier octet de la valeur.
fn position_valeur(ligne: &[u8], cle: &str) -> Option<usize> {
    let cle = cle.as_bytes();
    let mut i = 0usize;
    let mut dans_chaine = false;
    let mut echappe = false;
    let mut debut_chaine = 0usize;
    while i < ligne.len() {
        let o = ligne[i];
        if dans_chaine {
            if echappe {
                echappe = false;
            } else if o == b'\\' {
                echappe = true;
            } else if o == b'"' {
                // Chaine fermee. Est-ce la cle qu'on cherche, suivie de `:` ?
                let contenu = &ligne[debut_chaine..i];
                if contenu == cle {
                    let mut j = i + 1;
                    while j < ligne.len() && est_blanc(ligne[j]) {
                        j += 1;
                    }
                    if j < ligne.len() && ligne[j] == b':' {
                        j += 1;
                        while j < ligne.len() && est_blanc(ligne[j]) {
                            j += 1;
                        }
                        return Some(j);
                    }
                }
                dans_chaine = false;
            }
        } else if o == b'"' {
            dans_chaine = true;
            debut_chaine = i + 1;
        }
        i += 1;
    }
    None
}

/// La valeur texte du champ `cle`, sans les guillemets.
///
/// Les echappements ne sont pas interpretes : aucune valeur de ce protocole
/// n'en contient, et les interpreter serait du code a defendre pour rien.
pub fn champ_texte<'a>(ligne: &'a [u8], cle: &str) -> Option<&'a [u8]> {
    let debut = position_valeur(ligne, cle)?;
    if ligne.get(debut) != Some(&b'"') {
        return None;
    }
    let mut i = debut + 1;
    while i < ligne.len() {
        match ligne[i] {
            b'\\' => i += 2,
            b'"' => return Some(&ligne[debut + 1..i]),
            _ => i += 1,
        }
    }
    None
}

/// Ce qu'une lecture d'entier peut donner.
///
/// # Pourquoi trois cas et non deux
///
/// « Absent » et « present mais illisible » ne se traitent PAS pareil. Un
/// argument absent peut avoir une valeur par defaut raisonnable ; un argument
/// present et illisible n'en a aucune -- s'y replier reviendrait a executer
/// une commande que personne n'a ecrite.
///
/// L'epreuve `un_entier_qui_deborde_ne_se_replie_pas_sur_une_petite_valeur`
/// a attrape exactement cela : `"n":99999999999999999999` debordait, la
/// lecture rendait « rien », et `events tail` se repliait sur cent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LectureEntier {
    Absent,
    /// Present, mais pas un entier decimal representable.
    Invalide,
    Valeur(u64),
}

/// Lit le champ `cle` comme entier decimal non signe.
pub fn lit_entier(ligne: &[u8], cle: &str) -> LectureEntier {
    let Some(debut) = position_valeur(ligne, cle) else {
        return LectureEntier::Absent;
    };
    let mut i = debut;
    let mut valeur = 0u64;
    let mut chiffres = 0usize;
    while i < ligne.len() && ligne[i].is_ascii_digit() {
        let Some(dix) = valeur.checked_mul(10) else {
            return LectureEntier::Invalide;
        };
        let Some(somme) = dix.checked_add((ligne[i] - b'0') as u64) else {
            return LectureEntier::Invalide;
        };
        valeur = somme;
        chiffres += 1;
        i += 1;
    }
    if chiffres == 0 {
        // Le champ est la, mais sa valeur n'est pas un nombre : une chaine,
        // un booleen, un objet. C'est present-et-illisible, pas absent.
        return LectureEntier::Invalide;
    }
    LectureEntier::Valeur(valeur)
}

/// La valeur entiere du champ `cle`, quand la distinction ne sert pas.
pub fn champ_entier(ligne: &[u8], cle: &str) -> Option<u64> {
    match lit_entier(ligne, cle) {
        LectureEntier::Valeur(v) => Some(v),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// HEXADECIMAL
// ---------------------------------------------------------------------------

/// Decode de l'hexadecimal minuscule ou majuscule. Rend faux si la longueur
/// ne correspond pas exactement, ou si un caractere n'est pas hexadecimal.
pub fn hex_decode(src: &[u8], dst: &mut [u8]) -> bool {
    if src.len() != dst.len() * 2 {
        return false;
    }
    for (i, paire) in src.chunks_exact(2).enumerate() {
        let (Some(haut), Some(bas)) = (quartet(paire[0]), quartet(paire[1])) else {
            return false;
        };
        dst[i] = (haut << 4) | bas;
    }
    true
}

fn quartet(o: u8) -> Option<u8> {
    match o {
        b'0'..=b'9' => Some(o - b'0'),
        b'a'..=b'f' => Some(o - b'a' + 10),
        b'A'..=b'F' => Some(o - b'A' + 10),
        _ => None,
    }
}

/// Encode en hexadecimal minuscule. `dst` doit faire `2 * src.len()`.
pub fn hex_encode(src: &[u8], dst: &mut [u8]) -> bool {
    if dst.len() != src.len() * 2 {
        return false;
    }
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    for (i, &octet) in src.iter().enumerate() {
        dst[i * 2] = TABLE[(octet >> 4) as usize];
        dst[i * 2 + 1] = TABLE[(octet & 0x0F) as usize];
    }
    true
}

// ---------------------------------------------------------------------------
// L'AUTHENTIFICATION
// ---------------------------------------------------------------------------

/// Egalite a TEMPS CONSTANT.
///
/// # Pourquoi ce n'est pas une optimisation prematuree a l'envers
///
/// `a == b` sur des tranches s'arrete au premier octet different. Un
/// attaquant sur le segment local mesure ce temps, retient l'octet qui tarde
/// le plus, et recommence : trente-deux fois deux cent cinquante-six essais,
/// soit huit mille cent quatre-vingt-douze, au lieu de deux puissance deux
/// cent cinquante-six. C'est la difference entre « impossible » et « une
/// apres-midi ».
///
/// La boucle ici parcourt TOUJOURS les deux tranches entieres et n'a aucune
/// branche dependant du contenu.
pub fn egal_temps_constant(a: &[u8], b: &[u8]) -> bool {
    // La longueur, elle, n'est pas un secret : elle est fixee par le
    // protocole, et un attaquant la connait deja.
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0u8;
    for i in 0..a.len() {
        difference |= a[i] ^ b[i];
    }
    difference == 0
}

/// Le HMAC attendu pour ce nonce.
pub fn signature(jeton: &[u8], nonce: &[u8]) -> [u8; 32] {
    sha256::hmac(jeton, nonce)
}

/// Le HMAC rendu par le client est-il le bon ?
///
/// Un jeton VIDE refuse tout. Sans cela, une image construite sans
/// `BOUCHAUD_DEBUG_TOKEN` exposerait un debugger ouvert a quiconque atteint
/// le segment local -- exactement le contraire de ce qu'une absence de jeton
/// doit signifier.
pub fn verifie(jeton: &[u8], nonce: &[u8], rendu: &[u8]) -> bool {
    if jeton.is_empty() || rendu.len() != 32 {
        return false;
    }
    egal_temps_constant(&signature(jeton, nonce), rendu)
}

// ---------------------------------------------------------------------------
// L'ANALYSE D'UNE COMMANDE
// ---------------------------------------------------------------------------

/// Lit une ligne et rend la commande, ou dit pourquoi elle est refusee.
pub fn analyse(ligne: &[u8]) -> Result<Commande, Erreur> {
    if ligne.iter().all(|o| est_blanc(*o)) {
        return Err(Erreur::Vide);
    }
    let debut = ligne.iter().position(|o| !est_blanc(*o)).unwrap_or(0);
    if ligne[debut] != b'{' {
        return Err(Erreur::PasUnObjet);
    }
    let Some(cmd) = champ_texte(ligne, "cmd") else {
        return Err(Erreur::CommandeAbsente);
    };

    match cmd {
        b"hello" => {
            let Some(hex) = champ_texte(ligne, "mac") else {
                return Err(Erreur::ArgumentManquant);
            };
            let mut mac = [0u8; 32];
            if !hex_decode(hex, &mut mac) {
                return Err(Erreur::ArgumentInvalide);
            }
            Ok(Commande::Hello { mac })
        }
        b"status" => Ok(Commande::Status),
        b"audit status" => Ok(Commande::AuditStatus),
        b"audit run" => Ok(Commande::AuditRun),
        b"audit last" => Ok(Commande::AuditLast),
        b"net status" => Ok(Commande::NetStatus),
        b"rtl8168 status" => Ok(Commande::Rtl8168Status),
        b"rtl8168 ring" => Ok(Commande::Rtl8168Ring),
        b"rtl8168 desc" => {
            let n = match lit_entier(ligne, "n") {
                LectureEntier::Absent => return Err(Erreur::ArgumentManquant),
                LectureEntier::Invalide => return Err(Erreur::ArgumentInvalide),
                LectureEntier::Valeur(n) => n,
            };
            if n >= DESCRIPTEURS_MAX as u64 {
                return Err(Erreur::ArgumentInvalide);
            }
            Ok(Commande::Rtl8168Desc { index: n as u16 })
        }
        b"dhcp status" => Ok(Commande::DhcpStatus),
        b"blackbox status" => Ok(Commande::BlackboxStatus),
        b"blackbox checkpoint" => Ok(Commande::BlackboxCheckpoint),
        b"events tail" => {
            // `n` ABSENT vaut cent : une commande de confort ne doit pas
            // echouer sur un argument qu'on peut raisonnablement deviner.
            // `n` PRESENT ET ILLISIBLE est refuse : s'y replier reviendrait a
            // executer une commande que personne n'a ecrite.
            let combien = match lit_entier(ligne, "n") {
                LectureEntier::Absent => 100,
                LectureEntier::Invalide => return Err(Erreur::ArgumentInvalide),
                LectureEntier::Valeur(n) => n,
            };
            if combien == 0 || combien > EVENTS_TAIL_MAX as u64 {
                return Err(Erreur::ArgumentInvalide);
            }
            Ok(Commande::EventsTail { combien: combien as u32 })
        }
        b"events watch" => Ok(Commande::EventsWatch),
        b"services snapshot" => Ok(Commande::ServicesSnapshot),
        // BOUCHAUD_HOTFIX12_SERVICES_REMOTE_DETAIL_V1
        b"services page" => {
            let start = match lit_entier(ligne, "start") {
                LectureEntier::Absent => 0,
                LectureEntier::Invalide => return Err(Erreur::ArgumentInvalide),
                LectureEntier::Valeur(n) => n,
            };
            if start >= SERVICES_REMOTE_MAX as u64 {
                return Err(Erreur::ArgumentInvalide);
            }
            Ok(Commande::ServicesPage { start: start as u16 })
        }
        b"processes snapshot" => Ok(Commande::ProcessesSnapshot),
        b"memory snapshot" => Ok(Commande::MemorySnapshot),
        // BOUCHAUD_HOTFIX10_INTERNET_PROOF_CHAIN_V1
        b"internet proof start" => Ok(Commande::InternetProofStart),
        b"internet proof status" => Ok(Commande::InternetProofStatus),
        // BOUCHAUD_HOTFIX11_SERIAL_BRDP
        b"serial status" => Ok(Commande::SerialStatus),
        b"serial read" => {
            let start = match lit_entier(ligne, "start") {
                LectureEntier::Absent => return Err(Erreur::ArgumentManquant),
                LectureEntier::Invalide => return Err(Erreur::ArgumentInvalide),
                LectureEntier::Valeur(n) => n,
            };
            let combien = match lit_entier(ligne, "n") {
                LectureEntier::Absent => return Err(Erreur::ArgumentManquant),
                LectureEntier::Invalide => return Err(Erreur::ArgumentInvalide),
                LectureEntier::Valeur(n) => n,
            };
            if combien == 0 || combien > SERIAL_READ_MAX as u64 {
                return Err(Erreur::ArgumentInvalide);
            }
            Ok(Commande::SerialRead {
                start,
                combien: combien as u16,
            })
        }
        // BOUCHAUD_P0_REMOTE_CONTROL_V1
        b"system reboot" => Ok(Commande::SystemReboot),
        b"system shutdown" => Ok(Commande::SystemShutdown),
        b"browser start" => Ok(Commande::BrowserStart),
        b"browser stop" => Ok(Commande::BrowserStop),
        b"browser restart" => Ok(Commande::BrowserRestart),
        b"process kill" => {
            let pid = match lit_entier(ligne, "pid") {
                LectureEntier::Absent => return Err(Erreur::ArgumentManquant),
                LectureEntier::Invalide => return Err(Erreur::ArgumentInvalide),
                LectureEntier::Valeur(n) => n,
            };
            if pid <= 1 || pid > u32::MAX as u64 { return Err(Erreur::ArgumentInvalide); }
            Ok(Commande::ProcessKill { pid: pid as u32 })
        }
        b"process kill-tree" => {
            let pid = match lit_entier(ligne, "pid") {
                LectureEntier::Absent => return Err(Erreur::ArgumentManquant),
                LectureEntier::Invalide => return Err(Erreur::ArgumentInvalide),
                LectureEntier::Valeur(n) => n,
            };
            if pid <= 1 || pid > u32::MAX as u64 { return Err(Erreur::ArgumentInvalide); }
            Ok(Commande::ProcessKillTree { pid: pid as u32 })
        }
        b"quit" => Ok(Commande::Quit),
        // PAS DE REPLI SUR UN SHELL. Une commande inconnue est refusee, et
        // c'est tout : il n'y a rien derriere la liste blanche.
        _ => Err(Erreur::CommandeInconnue),
    }
}

/// Le nom d'une erreur, tel qu'il part sur le fil.
pub const fn nom_erreur(e: Erreur) -> &'static str {
    match e {
        Erreur::Vide => "vide",
        Erreur::PasUnObjet => "pas-un-objet",
        Erreur::CommandeAbsente => "commande-absente",
        Erreur::CommandeInconnue => "commande-inconnue",
        Erreur::ArgumentManquant => "argument-manquant",
        Erreur::ArgumentInvalide => "argument-invalide",
        Erreur::TropLongue => "ligne-trop-longue",
        Erreur::NonAuthentifie => "non-authentifie",
        Erreur::AuthRefusee => "auth-refusee",
        Erreur::FilePleine => "file-pleine",
    }
}
