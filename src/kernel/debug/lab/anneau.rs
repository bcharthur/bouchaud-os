//! L'anneau d'evenements du LAB : une seule source de verite.
//!
//! # Pourquoi un anneau, et pourquoi UN SEUL
//!
//! Les campagnes B2 et B3 ont mis l'observabilite critique dans
//! `serial_println!`. Le releve TRIGKEY dit ce que cela vaut :
//!
//! ```text
//! com1=bus-flottant  serial_bytes=0
//! ```
//!
//! Pas un octet. Toute la trace de ces campagnes a ete ecrite dans un port
//! serie qui n'existe pas sur cette machine. Ce n'est pas un defaut de
//! reglage : c'est une architecture ou le diagnostic depend d'un peripherique
//! qui peut ne pas etre la.
//!
//! La regle devient donc : une sonde emet dans CET anneau, et les
//! consommateurs -- boite noire, shell, debugger distant, telemetrie -- y
//! lisent. Aucun consommateur n'est sur le chemin de la sonde ; aucun ne peut
//! le bloquer ; l'absence de l'un n'efface rien pour les autres.
//!
//! ```text
//!   SONDE ──▶ ANNEAU ──┬──▶ boite noire
//!                      ├──▶ shell
//!                      ├──▶ debugger distant (BRDP)
//!                      └──▶ telemetrie best-effort
//! ```
//!
//! # Ce que « utilisable depuis un contexte bas niveau » interdit
//!
//! Une sonde peut se declencher sous `VERROU_RECEPTION`, dans un traitant
//! d'interruption, ou pendant une reprise de carte. D'ou :
//!
//!   - **pas d'allocation** : les emplacements sont un tableau statique ;
//!   - **pas de verrou bloquant** : un `fetch_add` reserve un numero, et
//!     l'ecriture d'un emplacement ne fait attendre personne ;
//!   - **borne** : l'anneau ecrase ses plus vieux evenements plutot que de
//!     grandir. Un lecteur en retard l'APPREND (`Lecture::Ecrasee`) au lieu
//!     de recevoir en silence un trou dans sa chronologie ;
//!   - **pas de `Mutex`, pas de `SpinLock`** : un emetteur qui prendrait un
//!     verrou tenu par un lecteur plus lent bloquerait le drainage reseau,
//!     c'est-a-dire precisement ce qu'on essaie d'observer.
//!
//! # Le protocole d'un emplacement
//!
//! Un sceau par emplacement, pas un verrou. L'ecrivain estampille, remplit,
//! puis valide. Le lecteur verifie que l'estampille ET la validation portent
//! encore SON numero apres avoir lu : si l'anneau a fait un tour pendant sa
//! lecture, il le voit et le dit.

use core::sync::atomic::{AtomicU64, Ordering};

/// Un evenement, tel qu'il sort de l'anneau.
///
/// Quatre arguments, et pas un de plus. Une sonde qui aurait besoin du
/// cinquieme emet un second evenement : c'est plus lisible qu'un champ
/// fourre-tout, et cela garde l'emplacement a une ligne de cache.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Evenement {
    /// Numero d'ordre global, a partir de 1. Il ne recule jamais et ne saute
    /// jamais : un trou dans la suite lue est une PERTE, et se voit.
    pub seq: u64,
    /// Horodatage monotone, en nanosecondes.
    pub t_ns: u64,
    /// Coeur qui a emis.
    pub cpu: u16,
    /// Famille de l'evenement. Voir `catalogue::Categorie`.
    pub categorie: u16,
    /// Identifiant dans le catalogue.
    pub event_id: u32,
    pub args: [u64; 4],
}

/// Ce qu'une lecture peut rendre.
///
/// Les trois refus sont distincts EXPRES. « Je n'ai rien » et « tu es arrive
/// trop tard » ne demandent pas la meme conduite au lecteur : le premier
/// attend, le second signale une perte et se recale.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lecture {
    /// L'evenement, entier.
    Evenement(Evenement),
    /// Cet evenement a ete ecrase : le lecteur etait trop lent.
    Ecrasee,
    /// Cet evenement n'a pas encore ete emis.
    PasEncore,
    /// Une ecriture courait pendant la lecture. Reessayer suffit.
    Dechiree,
}

/// Un emplacement : deux sceaux et la charge utile.
///
/// Soixante-quatre octets pile, l'`entete` empaquetant `cpu`, `categorie` et
/// `event_id`. Un emplacement ne chevauche donc jamais deux lignes de cache,
/// et deux emetteurs sur des coeurs differents ne se les volent pas.
#[repr(align(64))]
struct Emplacement {
    /// Numero que l'ecrivain a reserve. Zero : jamais servi.
    estampille: AtomicU64,
    /// Numero dont le contenu est COMPLET. Pose en dernier.
    valide: AtomicU64,
    t_ns: AtomicU64,
    /// `cpu << 48 | categorie << 32 | event_id`.
    entete: AtomicU64,
    args: [AtomicU64; 4],
}

impl Emplacement {
    const fn vide() -> Self {
        Self {
            estampille: AtomicU64::new(0),
            valide: AtomicU64::new(0),
            t_ns: AtomicU64::new(0),
            entete: AtomicU64::new(0),
            args: [
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ],
        }
    }
}

const fn empaquete(cpu: u16, categorie: u16, event_id: u32) -> u64 {
    ((cpu as u64) << 48) | ((categorie as u64) << 32) | event_id as u64
}

const fn depaquete(entete: u64) -> (u16, u16, u32) {
    (
        (entete >> 48) as u16,
        ((entete >> 32) & 0xFFFF) as u16,
        (entete & 0xFFFF_FFFF) as u32,
    )
}

/// L'anneau. `N` doit etre une puissance de deux.
pub struct Anneau<const N: usize> {
    emplacements: [Emplacement; N],
    /// Prochain numero a distribuer. Commence a UN : zero est le « jamais
    /// servi » des emplacements, et une valeur ne peut pas signifier les deux.
    suivant: AtomicU64,
}

impl<const N: usize> Anneau<N> {
    pub const fn nouveau() -> Self {
        // Une puissance de deux rend le modulo gratuit ET garantit qu'un tour
        // d'anneau ne desaligne jamais la correspondance numero/emplacement.
        assert!(N.is_power_of_two(), "la taille de l'anneau doit etre une puissance de deux");
        assert!(N >= 2);
        Self {
            emplacements: [const { Emplacement::vide() }; N],
            suivant: AtomicU64::new(1),
        }
    }

    pub const fn capacite(&self) -> usize {
        N
    }

    /// Nombre total d'evenements emis depuis l'amorcage.
    pub fn emises(&self) -> u64 {
        self.suivant.load(Ordering::Relaxed).saturating_sub(1)
    }

    /// Plus ancien numero encore lisible. Tout ce qui precede est ecrase.
    pub fn plus_ancienne(&self) -> u64 {
        let suivant = self.suivant.load(Ordering::Relaxed);
        suivant.saturating_sub(N as u64).max(1)
    }

    /// Prochain numero qui sera distribue.
    pub fn prochaine(&self) -> u64 {
        self.suivant.load(Ordering::Relaxed)
    }

    /// Evenements perdus parce qu'un lecteur etait en retard de plus d'un tour.
    ///
    /// Ce n'est PAS un compteur d'incident de l'anneau : un anneau borne qui
    /// tourne fait exactement son travail. C'est ce qu'un lecteur doit savoir
    /// pour ne pas presenter une chronologie trouee comme continue.
    pub fn perdues_pour(&self, depuis_seq: u64) -> u64 {
        self.plus_ancienne().saturating_sub(depuis_seq)
    }

    /// Emet un evenement. Ne bloque pas, n'alloue pas, ne peut pas echouer.
    ///
    /// Rend le numero attribue.
    pub fn emets(
        &self,
        t_ns: u64,
        cpu: u16,
        categorie: u16,
        event_id: u32,
        args: [u64; 4],
    ) -> u64 {
        let seq = self.suivant.fetch_add(1, Ordering::Relaxed);
        let e = &self.emplacements[(seq % N as u64) as usize];

        // 1. J'ANNONCE que cet emplacement m'appartient maintenant. Un lecteur
        //    qui voit une estampille plus recente que son numero sait, des cet
        //    instant, qu'il est arrive trop tard -- avant meme que le contenu
        //    change sous lui.
        e.estampille.store(seq, Ordering::Release);

        // 2. Je remplis. `Relaxed` suffit : la publication se fait en 3, et
        //    c'est elle que le lecteur acquiert.
        e.t_ns.store(t_ns, Ordering::Relaxed);
        e.entete.store(empaquete(cpu, categorie, event_id), Ordering::Relaxed);
        e.args[0].store(args[0], Ordering::Relaxed);
        e.args[1].store(args[1], Ordering::Relaxed);
        e.args[2].store(args[2], Ordering::Relaxed);
        e.args[3].store(args[3], Ordering::Relaxed);

        // 3. JE PUBLIE. Tant que ce sceau n'est pas pose, aucun lecteur
        //    n'accepte le contenu.
        e.valide.store(seq, Ordering::Release);
        seq
    }

    /// Lit l'evenement numero `seq`.
    pub fn lis(&self, seq: u64) -> Lecture {
        if seq == 0 {
            return Lecture::Ecrasee;
        }
        if seq >= self.suivant.load(Ordering::Acquire) {
            return Lecture::PasEncore;
        }
        let e = &self.emplacements[(seq % N as u64) as usize];

        let estampille = e.estampille.load(Ordering::Acquire);
        if estampille > seq {
            return Lecture::Ecrasee;
        }
        if estampille < seq || e.valide.load(Ordering::Acquire) != seq {
            // L'ecrivain a reserve le numero mais n'a pas fini. Ce n'est pas
            // une perte : c'est une course, et reessayer la gagne.
            return Lecture::Dechiree;
        }

        let t_ns = e.t_ns.load(Ordering::Relaxed);
        let (cpu, categorie, event_id) = depaquete(e.entete.load(Ordering::Relaxed));
        let args = [
            e.args[0].load(Ordering::Relaxed),
            e.args[1].load(Ordering::Relaxed),
            e.args[2].load(Ordering::Relaxed),
            e.args[3].load(Ordering::Relaxed),
        ];

        // LE SECOND REGARD. Sans lui, un tour d'anneau survenu pendant la
        // lecture rendrait un evenement mi-ancien mi-neuf, avec un numero qui
        // le fait passer pour entier -- le pire des resultats possibles, parce
        // qu'il est credible.
        if e.valide.load(Ordering::Acquire) != seq || e.estampille.load(Ordering::Acquire) != seq {
            return Lecture::Dechiree;
        }

        Lecture::Evenement(Evenement { seq, t_ns, cpu, categorie, event_id, args })
    }

    /// Lit en reessayant les dechirures. Rend `None` sur perte ou absence.
    ///
    /// Le nombre d'essais est BORNE : un lecteur qui boucle jusqu'au succes
    /// sous un emetteur rapide ne rendrait jamais la main, et ce module existe
    /// pour ne bloquer personne.
    pub fn lis_stable(&self, seq: u64, essais: u32) -> Lecture {
        let mut lecture = Lecture::Dechiree;
        for _ in 0..essais.max(1) {
            lecture = self.lis(seq);
            if lecture != Lecture::Dechiree {
                return lecture;
            }
        }
        lecture
    }

    /// Recale un curseur sur le plus ancien evenement encore lisible.
    ///
    /// Rend le nouveau curseur et le nombre d'evenements perdus. Un lecteur
    /// appelle ceci quand `lis` lui a rendu `Ecrasee` : la suite qu'il
    /// presentera aura un trou, et il doit pouvoir le CHIFFRER.
    pub fn recale(&self, curseur: u64) -> (u64, u64) {
        let plus_ancienne = self.plus_ancienne();
        if curseur >= plus_ancienne {
            return (curseur, 0);
        }
        (plus_ancienne, plus_ancienne - curseur)
    }
}
