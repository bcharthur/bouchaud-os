//! Ou passe le temps d'un processus qui tourne sans avancer.
//!
//! BOUCHAUD_PROFIL_APPELS_PAR_FENETRE_V1
//!
//! # Le defaut que ceci corrige
//!
//! Le releve physique du 15 septembre 2026 dit :
//!
//! ```text
//! [PROC-SAMPLE] ... name=WebContent cpu_pct=76 ctx_delta=4 ...
//! ```
//!
//! Soixante-seize pour cent d'un coeur pour QUATRE changements de contexte en
//! trente secondes. Un processus qui travaille change de contexte ; celui-la
//! tourne sur place. Mais le releve s'arrete la : il dit qu'on brule un coeur,
//! pas a quoi.
//!
//! Le noyau comptait pourtant deja chaque appel systeme, par numero, depuis le
//! demarrage -- `SYSCALL_HITS`. Ce compte n'etait lisible que par la commande
//! interactive `syscalls`, c'est-a-dire au clavier. Sur la machine de
//! reference, le clavier ne repond pas : la mesure existait et n'atteignait
//! jamais l'archive.
//!
//! # Pourquoi des DELTAS, et pas le cumul
//!
//! Le cumul depuis le demarrage est domine par le demarrage. Un navigateur qui
//! se met a tourner en rond au bout de deux minutes n'y deplace pas le
//! classement : ses millions d'appels arrivent apres que des millions d'autres
//! ont deja ete comptes.
//!
//! Ce qui designe un tour en rond, c'est le nombre d'appels PAR FENETRE, mis
//! en regard de la duree de la fenetre. Quatre millions de `clock_gettime` en
//! trente secondes n'a pas d'autre lecture possible.
//!
//! # Pourquoi compter aussi les `EAGAIN`
//!
//! Une boucle d'attente active ne se reconnait pas a l'appel qu'elle emet mais
//! a sa REPONSE : elle interroge un descripteur non bloquant qui repond
//! toujours « rien pour l'instant ». `lire=1 200 000` ne dit pas s'il y a du
//! travail ; `lire=1 200 000 eagain=1 199 998` le dit.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Nombre de numeros suivis. Doit rester egal a `SYSCALL_HITS_LEN`.
pub const NUMEROS: usize = 336;

static EAGAIN: [AtomicU32; NUMEROS] = [const { AtomicU32::new(0) }; NUMEROS];
static HITS_PRECEDENTS: [AtomicU32; NUMEROS] = [const { AtomicU32::new(0) }; NUMEROS];
static EAGAIN_PRECEDENTS: [AtomicU32; NUMEROS] = [const { AtomicU32::new(0) }; NUMEROS];
static FENETRE_PRECEDENTE_NS: AtomicU64 = AtomicU64::new(0);

/// Un appel a repondu « rien pour l'instant ».
#[inline]
pub fn note_eagain(numero: u64) {
    EAGAIN[(numero as usize).min(NUMEROS - 1)].fetch_add(1, Ordering::Relaxed);
}

/// Combien de fois cet appel a repondu `EAGAIN` depuis le demarrage.
pub fn eagain(numero: u64) -> u32 {
    EAGAIN[(numero as usize).min(NUMEROS - 1)].load(Ordering::Relaxed)
}

/// Ce qu'un appel a coute sur la fenetre qui vient de s'ecouler.
#[derive(Clone, Copy)]
pub struct Chaud {
    pub numero: u64,
    pub appels: u32,
    pub eagain: u32,
}

/// Duree de la fenetre, en nanosecondes, et remise a zero du repere.
///
/// Rend zero a la toute premiere fenetre : il n'y a alors rien a comparer, et
/// un debit calcule sur une duree inventee serait pire qu'absent.
pub fn ferme_fenetre(maintenant_ns: u64) -> u64 {
    let precedent = FENETRE_PRECEDENTE_NS.swap(maintenant_ns, Ordering::Relaxed);
    if precedent == 0 || maintenant_ns <= precedent {
        0
    } else {
        maintenant_ns - precedent
    }
}

/// Insere une entree dans un classement borne, trie par appels decroissants.
///
/// `poses` est le nombre de places deja occupees ; la valeur rendue est le
/// nouveau. Une entree plus froide que la derniere place d'un classement plein
/// est ignoree -- c'est tout l'interet d'un top N : il ne grandit pas.
///
/// Cette fonction est separee de [`plus_chauds`] parce qu'elle est la seule
/// partie difficile, et qu'elle ne depend d'aucun etat global : elle se
/// verifie seule sur l'hote.
pub fn insere(sortie: &mut [Chaud], poses: usize, entree: Chaud) -> usize {
    if sortie.is_empty() {
        return 0;
    }
    let mut place = poses.min(sortie.len());
    while place > 0 && sortie[place - 1].appels < entree.appels {
        if place < sortie.len() {
            sortie[place] = sortie[place - 1];
        }
        place -= 1;
    }
    if place >= sortie.len() {
        // Plus froide que tout ce qui est deja classe, et le classement est
        // plein : elle n'a pas sa place.
        return poses;
    }
    sortie[place] = entree;
    (poses + 1).min(sortie.len())
}

/// Les appels les plus emis depuis le dernier passage, du plus chaud au moins.
///
/// Le classement est fait sur place dans `sortie`, sans allocation : cette
/// fonction est appelee depuis le releve periodique, qui tourne sur la pile du
/// gestionnaire de fenetres.
///
/// `lire_hits` donne le compte cumule d'un numero. Le passer en argument evite
/// a ce fichier de connaitre la table du dispatch, et le rend verifiable seul.
pub fn plus_chauds(sortie: &mut [Chaud], lire_hits: impl Fn(usize) -> u32) -> (usize, u64) {
    let mut poses = 0usize;
    let mut total = 0u64;
    for numero in 0..NUMEROS {
        let cumul = lire_hits(numero);
        let avant = HITS_PRECEDENTS[numero].swap(cumul, Ordering::Relaxed);
        let appels = cumul.wrapping_sub(avant);
        let cumul_eagain = EAGAIN[numero].load(Ordering::Relaxed);
        let avant_eagain = EAGAIN_PRECEDENTS[numero].swap(cumul_eagain, Ordering::Relaxed);
        if appels == 0 {
            continue;
        }
        total += appels as u64;
        poses = insere(
            sortie,
            poses,
            Chaud {
                numero: numero as u64,
                appels,
                eagain: cumul_eagain.wrapping_sub(avant_eagain),
            },
        );
    }
    (poses, total)
}
