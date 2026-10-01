//! La configuration IPv4 d'eth0, publiee par generation.
//!
//! BOUCHAUD_NET_CONFIG_GENERATION_V1
//!
//! # Ce qui etait faux
//!
//! L'adresse, la passerelle, le resolveur, le masque et le nom du reseau
//! etaient six `static mut` ecrits champ par champ, en DEUX appels
//! (`set_config(ip, gw, dns)` puis `pose_identite_reseau(nom, masque)`), et lus
//! un par un. Rien ne les a jamais proteges : le fil DHCP n'a pas pris le gros
//! verrou. Un lecteur pouvait donc voir l'adresse du nouveau bail avec le
//! masque de l'ancien -- `same_subnet` decidait alors du prochain saut avec
//! deux reseaux a la fois -- ou, dans `smol_tcp`, la nouvelle adresse et
//! l'ancienne passerelle. Et deux ecrivains (veilleur de lien, banc RX,
//! demarrage) faisaient chacun des lecture-modification-ecriture
//! (« efface la presomption SLIRP si elle est encore la ») sans exclusion.
//!
//! # Qui ecrit, qui lit, depuis ou
//!
//!   * ecrivains : fils noyau seulement (demarrage, `net-lien`, banc RX,
//!     client DHCP qu'ils appellent). Aucun en interruption : les deux pilotes
//!     fonctionnent en polling ;
//!   * lecteurs : pompe RX (`route_ipv4`), emission (`hop_mac`), smoltcp,
//!     appels systeme (`getsockname`), interface, releves, et la boite noire
//!     -- qui peut lire en contexte de faute, ou rien ne doit bloquer.
//!
//! # Le mecanisme
//!
//! Un instantane versionne. La sequence est impaire pendant une ecriture ; un
//! lecteur lit la sequence, les champs, puis la sequence encore, et
//! recommence si elle a bouge ou etait impaire. Les champs sont des atomiques
//! (pas de course de donnees au sens du modele memoire) ; l'ordre est donne
//! par les barrieres Release/Acquire. Les lecteurs ne prennent AUCUN verrou :
//! la pompe RX et la boite noire ne peuvent pas se retrouver derriere un
//! ecrivain.
//!
//! Les ecrivains sont serialises par `ECRIVAINS`, un `SpinLockIrq` propre a
//! cette configuration -- pas un verrou global. Il coupe les interruptions :
//! sur un seul coeur, un lecteur ne peut pas tourner indefiniment derriere un
//! ecrivain preempte au milieu de sa fenetre impaire. Toute modification passe
//! par `modifie`, qui lit l'etat courant SOUS ce verrou : les
//! lecture-modification-ecriture des ecrivains sont atomiques entre elles.
//!
//! La fermeture passee a `modifie` ne doit ni dormir, ni journaliser, ni
//! appeler `instantane()` : elle recoit deja l'etat courant.
//!
//! Preuve hote : tools/smp/test_config_reseau_coherente.rs (l'ancien protocole
//! melange des generations, celui-ci jamais). Preuve QEMU : `netcfg-banc`.

use core::sync::atomic::{fence, AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};

use super::ipv4::Ipv4Addr;
use crate::kernel::sync::SpinLockIrq;

/// Longueur maximale du nom de reseau retenu (option DHCP 15).
pub const NOM_MAX: usize = 63;

/// Une configuration complete, telle qu'une seule publication l'a posee.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ConfigEth0 {
    pub ip: Ipv4Addr,
    pub passerelle: Ipv4Addr,
    pub resolveur: Ipv4Addr,
    pub masque: Ipv4Addr,
    nom: [u8; NOM_MAX],
    nom_len: u8,
}

impl ConfigEth0 {
    pub const fn neuve(ip: Ipv4Addr, passerelle: Ipv4Addr, resolveur: Ipv4Addr) -> Self {
        Self { ip, passerelle, resolveur, masque: [0; 4], nom: [0; NOM_MAX], nom_len: 0 }
    }

    /// Le nom annonce par DHCP (option 15), ou vide.
    pub fn nom(&self) -> &[u8] {
        &self.nom[..self.nom_len as usize]
    }

    /// Retient le nom et le masque que le bail a appris.
    pub fn pose_identite(&mut self, nom: &[u8], masque: Ipv4Addr) {
        let n = nom.len().min(NOM_MAX);
        self.nom = [0; NOM_MAX];
        self.nom[..n].copy_from_slice(&nom[..n]);
        self.nom_len = n as u8;
        self.masque = masque;
    }

    /// Le lien est tombe : l'identite du reseau ne vaut plus rien.
    pub fn oublie_identite(&mut self) {
        self.nom = [0; NOM_MAX];
        self.nom_len = 0;
        self.masque = [0; 4];
    }

    /// `ip` est-elle sur le sous-reseau de CETTE configuration ? Masque et
    /// adresse viennent du meme instantane, par construction.
    pub fn meme_sous_reseau(&self, ip: &Ipv4Addr) -> bool {
        if self.masque == [0, 0, 0, 0] {
            // Sans masque connu -- avant le bail, ou sur une configuration
            // statique de repli -- le /24, comme avant.
            return ip[0] == self.ip[0] && ip[1] == self.ip[1] && ip[2] == self.ip[2];
        }
        (0..4).all(|i| ip[i] & self.masque[i] == self.ip[i] & self.masque[i])
    }
}

const fn mot(a: Ipv4Addr) -> u32 {
    u32::from_be_bytes(a)
}

fn adresse(m: u32) -> Ipv4Addr {
    m.to_be_bytes()
}

/// Une configuration publiee par generation : le MECANISME, separe de son
/// instance. `ETH0` porte la configuration reelle ; `netcfg-banc` eprouve le
/// meme code sur sa propre instance, sans jamais toucher au reseau.
pub struct Publication {
    sequence: AtomicU64,
    ip: AtomicU32,
    passerelle: AtomicU32,
    resolveur: AtomicU32,
    masque: AtomicU32,
    nom: [AtomicU8; NOM_MAX],
    nom_len: AtomicU8,
    /// Verrou des ECRIVAINS de cette publication, et d'elle seule.
    ecrivains: SpinLockIrq<()>,
    /// Lectures recommencees parce qu'une ecriture etait en cours.
    relectures: AtomicU64,
}

impl Publication {
    /// `depart` ne porte pas de nom : une configuration initiale n'en a pas.
    pub const fn new(depart: ConfigEth0) -> Self {
        Self {
            sequence: AtomicU64::new(0),
            ip: AtomicU32::new(mot(depart.ip)),
            passerelle: AtomicU32::new(mot(depart.passerelle)),
            resolveur: AtomicU32::new(mot(depart.resolveur)),
            masque: AtomicU32::new(mot(depart.masque)),
            nom: [const { AtomicU8::new(0) }; NOM_MAX],
            nom_len: AtomicU8::new(0),
            ecrivains: SpinLockIrq::new(()),
            relectures: AtomicU64::new(0),
        }
    }

    fn lit_champs(&self) -> ConfigEth0 {
        let mut nom = [0u8; NOM_MAX];
        for (i, octet) in nom.iter_mut().enumerate() {
            *octet = self.nom[i].load(Ordering::Relaxed);
        }
        ConfigEth0 {
            ip: adresse(self.ip.load(Ordering::Relaxed)),
            passerelle: adresse(self.passerelle.load(Ordering::Relaxed)),
            resolveur: adresse(self.resolveur.load(Ordering::Relaxed)),
            masque: adresse(self.masque.load(Ordering::Relaxed)),
            nom,
            nom_len: self.nom_len.load(Ordering::Relaxed).min(NOM_MAX as u8),
        }
    }

    fn ecrit_champs(&self, c: &ConfigEth0) {
        self.ip.store(mot(c.ip), Ordering::Relaxed);
        self.passerelle.store(mot(c.passerelle), Ordering::Relaxed);
        self.resolveur.store(mot(c.resolveur), Ordering::Relaxed);
        self.masque.store(mot(c.masque), Ordering::Relaxed);
        for (i, octet) in c.nom.iter().enumerate() {
            self.nom[i].store(*octet, Ordering::Relaxed);
        }
        self.nom_len.store(c.nom_len, Ordering::Relaxed);
    }

    /// Un instantane coherent : tous les champs viennent de la MEME publication.
    pub fn instantane(&self) -> ConfigEth0 {
        loop {
            if let Some(c) = self.essaie_une_lecture() {
                return c;
            }
            self.relectures.fetch_add(1, Ordering::Relaxed);
            core::hint::spin_loop();
        }
    }

    /// Comme `instantane`, mais renonce apres `essais` tentatives.
    pub fn instantane_borne(&self, essais: u32) -> Option<ConfigEth0> {
        (0..essais.max(1)).find_map(|_| self.essaie_une_lecture())
    }

    fn essaie_une_lecture(&self) -> Option<ConfigEth0> {
        let avant = self.sequence.load(Ordering::Acquire);
        if avant & 1 == 1 {
            return None;
        }
        let c = self.lit_champs();
        fence(Ordering::Acquire);
        (self.sequence.load(Ordering::Relaxed) == avant).then_some(c)
    }

    /// Seule porte d'ecriture. `f` recoit l'etat courant, lu sous le verrou
    /// des ecrivains, et le modifie ; une seule publication en sort, quel que
    /// soit le nombre de champs touches. Rien n'est publie si rien n'a change.
    pub fn modifie<R>(&self, f: impl FnOnce(&mut ConfigEth0) -> R) -> R {
        let _seul = self.ecrivains.lock();
        let avant = self.lit_champs();
        let mut c = avant;
        let rendu = f(&mut c);
        if c != avant {
            let s = self.sequence.load(Ordering::Relaxed);
            self.sequence.store(s + 1, Ordering::Relaxed);
            fence(Ordering::Release);
            self.ecrit_champs(&c);
            self.sequence.store(s + 2, Ordering::Release);
        }
        rendu
    }

    /// Nombre de publications, et de lectures recommencees.
    pub fn compteurs(&self) -> (u64, u64) {
        (self.sequence.load(Ordering::Relaxed) / 2, self.relectures.load(Ordering::Relaxed))
    }
}

/// La configuration REELLE d'eth0 : presomption SLIRP au depart, que DHCP
/// remplace.
static ETH0: Publication = Publication::new(ConfigEth0::neuve(
    super::presomption_slirp::IP,
    super::presomption_slirp::PASSERELLE,
    super::presomption_slirp::RESOLVEUR,
));

/// Un instantane coherent de la configuration d'eth0.
pub fn instantane() -> ConfigEth0 {
    ETH0.instantane()
}

/// Lecture bornee, pour la boite noire : elle peut lire alors qu'un ecrivain a
/// ete interrompu par une faute au milieu de sa fenetre.
pub fn instantane_borne(essais: u32) -> Option<ConfigEth0> {
    ETH0.instantane_borne(essais)
}

/// Seule porte d'ecriture de la configuration d'eth0. La fermeture ne doit ni
/// dormir, ni journaliser, ni appeler `instantane()`.
pub fn modifie<R>(f: impl FnOnce(&mut ConfigEth0) -> R) -> R {
    ETH0.modifie(f)
}

/// Publications d'eth0 depuis l'amorcage, et lectures recommencees.
pub fn compteurs() -> (u64, u64) {
    ETH0.compteurs()
}

// ---------------------------------------------------------------------------
// `netcfg-banc` : la preuve sous charge, dans le noyau.
// ---------------------------------------------------------------------------
//
// Sur l'instance `BANC` (meme code que `ETH0`), deux ecrivains publient des
// generations dont CHAQUE champ porte le numero ;
// des lecteurs prennent des instantanes et verifient que tous les champs
// portent le meme. En parallele, les memes lecteurs lisent aussi les champs
// SANS la sequence : ce temoin doit voir des melanges sur plusieurs coeurs,
// sinon le banc ne toucherait pas la fenetre et ne prouverait rien.

const BANC_LECTEURS_MAX: usize = 6;
/// L'instance du banc : le MEME code que `ETH0`, sans toucher au reseau. La
/// premiere version ecrivait dans la configuration reelle ; le veilleur de lien
/// y a publie son bail en plein banc (SMP4/8 : faux melanges), puis le banc a
/// « restaure » la presomption SLIRP par-dessus le bail.
static BANC: Publication = Publication::new(ConfigEth0::neuve([0; 4], [0; 4], [0; 4]));
static BANC_ACTIF: AtomicBool = AtomicBool::new(false);
static BANC_LECTEURS: AtomicU32 = AtomicU32::new(0);
static BANC_DUREE_MS: AtomicU64 = AtomicU64::new(0);
static BANC_LECTURES: AtomicU64 = AtomicU64::new(0);
static BANC_MELANGES: AtomicU64 = AtomicU64::new(0);
static BANC_TEMOIN_LECTURES: AtomicU64 = AtomicU64::new(0);
static BANC_TEMOIN_MELANGES: AtomicU64 = AtomicU64::new(0);
static BANC_PUBLICATIONS: AtomicU64 = AtomicU64::new(0);
static BANC_PARTIS: AtomicU32 = AtomicU32::new(0);
static BANC_FINIS: AtomicU32 = AtomicU32::new(0);
static BANC_INDICE: AtomicU32 = AtomicU32::new(0);

fn generation(n: u32) -> ConfigEth0 {
    let (a, b) = ((n >> 8) as u8, n as u8);
    let mut c = ConfigEth0::neuve([10, a, b, 15], [10, a, b, 2], [10, a, b, 3]);
    c.pose_identite(&[b'g', a, b], [255, a, b, 0]);
    c
}

/// Tous les champs portent-ils la meme generation ?
fn coherente(c: &ConfigEth0) -> bool {
    let (a, b) = (c.ip[1], c.ip[2]);
    c.passerelle[1..3] == [a, b]
        && c.resolveur[1..3] == [a, b]
        && c.masque[1..3] == [a, b]
        && c.nom() == [b'g', a, b]
}

/// Un fil du banc : les deux premiers ecrivent, les suivants lisent. Il vit le
/// temps d'UN banc et se termine -- `run_noyau`, qui porte le banc depuis
/// l'autorun, tue de toute facon ce qui lui survit.
fn fil_banc() -> ! {
    let indice = BANC_INDICE.fetch_add(1, Ordering::AcqRel);
    BANC_PARTIS.fetch_add(1, Ordering::AcqRel);
    while !BANC_ACTIF.load(Ordering::Acquire) && BANC_FINIS.load(Ordering::Acquire) == 0 {
        core::hint::spin_loop();
        crate::kernel::task::yield_now();
    }
    if indice < 2 {
        // Ecrivain : generations paires pour l'un, impaires pour l'autre.
        let mut n = indice;
        while BANC_ACTIF.load(Ordering::Acquire) {
            let g = generation(n);
            BANC.modifie(|c| *c = g);
            BANC_PUBLICATIONS.fetch_add(1, Ordering::Relaxed);
            n = n.wrapping_add(2) & 0xFFFF;
            // Ceder regulierement : sur un seul coeur, le pilote doit pouvoir
            // se reveiller pour arreter le banc.
            if n % 64 < 2 {
                crate::kernel::task::yield_now();
            }
        }
    } else {
        let (mut lues, mut melangees, mut nues, mut nues_melangees) = (0u64, 0u64, 0u64, 0u64);
        while BANC_ACTIF.load(Ordering::Acquire) {
            lues += 1;
            if !coherente(&BANC.instantane()) {
                melangees += 1;
            }
            nues += 1;
            if !coherente(&BANC.lit_champs()) {
                nues_melangees += 1;
            }
            if lues % 256 == 0 {
                crate::kernel::task::yield_now();
            }
        }
        BANC_LECTURES.fetch_add(lues, Ordering::Relaxed);
        BANC_MELANGES.fetch_add(melangees, Ordering::Relaxed);
        BANC_TEMOIN_LECTURES.fetch_add(nues, Ordering::Relaxed);
        BANC_TEMOIN_MELANGES.fetch_add(nues_melangees, Ordering::Relaxed);
    }
    BANC_FINIS.fetch_add(1, Ordering::AcqRel);
    crate::kernel::task::exit_current(0)
}

/// Les bancs demandes, en couples (duree_ms, lecteurs). Remplie par `banc`.
const SUITE_MAX: usize = 8;
static SUITE: [AtomicU64; SUITE_MAX] = [const { AtomicU64::new(0) }; SUITE_MAX];
static SUITE_LONGUEUR: AtomicU32 = AtomicU32::new(0);

/// Le pilote, quand le banc est lance depuis le contexte d'AMORCAGE : il
/// deroule toute la suite, puis ETEINT la machine avec un code explicite.
///
/// Il ne rend pas la main par `exit_current` : en SMP1, `run_noyau` n'a pas
/// repris le contexte d'amorcage apres la sortie de ce fil (machine au repos
/// jusqu'au plafond, code de sortie 0 comme 1). C'est un defaut de
/// l'ordonnanceur, consigne pour son propre lot ; le banc ne doit pas en
/// dependre.
fn pilote_banc() -> ! {
    let n = SUITE_LONGUEUR.load(Ordering::Acquire) as usize;
    let mut reussis = 0;
    for i in 0..n {
        let couple = SUITE[i].load(Ordering::Acquire);
        BANC_DUREE_MS.store(couple >> 32, Ordering::Release);
        BANC_LECTEURS.store(couple as u32, Ordering::Release);
        if deroule_banc() == 0 {
            reussis += 1;
        }
    }
    let ok = n > 0 && reussis == n;
    crate::serial_println!("[NET-GEN-SUITE] v=1 {} reussis={}/{}", if ok { "OK" } else { "FAIL" }, reussis, n);
    crate::kernel::power::shutdown_avec_raison(
        if ok { crate::kernel::power::EXIT_OK } else { crate::kernel::power::EXIT_FAIL },
        "netcfg_banc_termine",
    )
}

fn attends<F: Fn() -> bool>(condition: F, limite_ms: u64) -> bool {
    let debut = crate::kernel::timer::monotonic_ms();
    while !condition() {
        if crate::kernel::timer::monotonic_ms().saturating_sub(debut) > limite_ms {
            return false;
        }
        crate::kernel::task::sleep_ticks(1);
    }
    true
}

fn deroule_banc() -> i32 {
    let lecteurs = BANC_LECTEURS.load(Ordering::Acquire);
    let duree_ms = BANC_DUREE_MS.load(Ordering::Acquire);
    let fils = 2 + lecteurs;
    for compteur in [&BANC_LECTURES, &BANC_MELANGES, &BANC_TEMOIN_LECTURES,
                     &BANC_TEMOIN_MELANGES, &BANC_PUBLICATIONS] {
        compteur.store(0, Ordering::Relaxed);
    }
    BANC_INDICE.store(0, Ordering::Release);
    BANC_PARTIS.store(0, Ordering::Release);
    BANC_FINIS.store(0, Ordering::Release);
    BANC_ACTIF.store(false, Ordering::Release);
    let (_, relectures_avant) = BANC.compteurs();

    for _ in 0..fils {
        if !crate::kernel::task::spawn_noyau_priorite(
            fil_banc, "netcfg-banc", crate::kernel::task::Priorite::Normale,
        ) {
            crate::serial_println!("[NET-GEN] v=1 FAIL raison=fil_refuse");
            return 1;
        }
    }
    if !attends(|| BANC_PARTIS.load(Ordering::Acquire) >= fils, 5_000) {
        crate::serial_println!("[NET-GEN] v=1 FAIL raison=fils_absents partis={}/{}",
            BANC_PARTIS.load(Ordering::Acquire), fils);
        BANC_FINIS.store(u32::MAX, Ordering::Release);
        return 1;
    }
    // Une generation synthetique AVANT d'ouvrir le banc : sans elle, un lecteur
    // elu avant le premier ecrivain lisait la configuration REELLE, que la
    // verification compte comme un melange (SMP1 : 256 faux melanges, zero
    // relecture -- aucune ecriture n'etait en cours).
    BANC.modifie(|c| *c = generation(0xFFFF));
    BANC_ACTIF.store(true, Ordering::Release);
    let _ = attends(|| false, duree_ms);
    BANC_ACTIF.store(false, Ordering::Release);
    let tous_finis = attends(|| BANC_FINIS.load(Ordering::Acquire) >= fils, 5_000);
    let (_, relectures_apres) = BANC.compteurs();
    let lectures = BANC_LECTURES.load(Ordering::Relaxed);
    let melanges = BANC_MELANGES.load(Ordering::Relaxed);
    let publications = BANC_PUBLICATIONS.load(Ordering::Relaxed);
    let ok = tous_finis && melanges == 0 && lectures > 0 && publications > 0;
    crate::serial_println!(
        "[NET-GEN] v=1 {} cpus={} ecrivains=2 lecteurs={} duree_ms={} publications={} \
lectures={} melanges={} relectures={} temoin_lectures={} temoin_melanges={} \
fils_finis={}",
        if ok { "OK" } else { "FAIL" },
        crate::arch::x86_64::smp::schedulable_cpus(),
        lecteurs, duree_ms, publications, lectures, melanges,
        relectures_apres.saturating_sub(relectures_avant),
        BANC_TEMOIN_LECTURES.load(Ordering::Relaxed),
        BANC_TEMOIN_MELANGES.load(Ordering::Relaxed),
        tous_finis as u8,
    );
    if ok { 0 } else { 1 }
}

/// `netcfg-banc ms lecteurs [ms lecteurs ...]` : rend 0 si aucun instantane
/// n'a melange deux generations. Le banc travaille sur sa propre instance ; la
/// configuration reelle d'eth0 n'est jamais ecrite.
///
/// Depuis une tache (shell interactif), les bancs se deroulent sur place.
/// Depuis l'autorun, le shell tourne dans le contexte d'AMORCAGE, sans tache
/// courante : on n'y peut ni dormir ni laisser courir d'autres fils (SMP1 :
/// `fils_absents`, puis panique `aucune tache active` sur `sleep_ticks`). La
/// suite s'y deroule donc dans un fil pilote, par `run_noyau`, qui ETEINT la
/// machine a la fin (voir `pilote_banc`) : c'est la derniere commande d'un
/// scenario.
pub fn banc(couples: &[(u64, u32)]) -> i32 {
    let couples = &couples[..couples.len().min(SUITE_MAX)];
    for (i, (ms, lecteurs)) in couples.iter().enumerate() {
        let ms = (*ms).clamp(100, 60_000);
        let lecteurs = (*lecteurs).clamp(1, BANC_LECTEURS_MAX as u32);
        SUITE[i].store((ms << 32) | lecteurs as u64, Ordering::Release);
    }
    SUITE_LONGUEUR.store(couples.len() as u32, Ordering::Release);
    if crate::kernel::task::in_user_task() {
        let mut code = 0;
        for i in 0..couples.len() {
            let couple = SUITE[i].load(Ordering::Acquire);
            BANC_DUREE_MS.store(couple >> 32, Ordering::Release);
            BANC_LECTEURS.store(couple as u32, Ordering::Release);
            code |= deroule_banc();
        }
        return code;
    }
    let _ = crate::kernel::task::run_noyau(pilote_banc, "netcfg-banc");
    // Atteint seulement si `run_noyau` rend la main avant la fin du pilote.
    crate::serial_println!("[NET-GEN-SUITE] v=1 FAIL raison=retour_anticipe");
    1
}
