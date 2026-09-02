//! Le protocole de parking sans verrou, isole pour etre demontrable.
//!
//! # Ce qu'il remplace
//!
//! Une file d'attente du noyau enregistrait son dormeur, relisait sa
//! generation, puis le declarait bloque -- le tout sous gros verrou. L'ordre
//! n'etait correct que grace a lui : il empechait tout reveilleur de tourner
//! entre les deux derniers pas.
//!
//! Sans verrou, la fenetre est reelle et le symptome est le pire possible :
//!
//! ```text
//! dormeur      relit la generation, ne voit rien de neuf
//! reveilleur   incremente la generation, voit waiters > 0,
//!              tente Bloque -> Pret : ECHOUE, le dormeur est encore Pret
//! dormeur      se declare bloque, et dort pour toujours
//! ```
//!
//! Une tache qui ne repart jamais ne produit aucun message : elle se voit
//! seulement comme une interface qui se fige.
//!
//! # L'ordre correct
//!
//! On publie `Blocked` D'ABORD, on relit la generation ENSUITE :
//!
//! ```text
//! dormeur      waiters++, publie Blocked, relit la generation
//! reveilleur   generation++, lit waiters, tente Bloque -> Pret
//! ```
//!
//! Chaque cote fait une ecriture puis une lecture, croisees, toutes deux en
//! ordre SEQUENTIEL. Deux ecritures ne peuvent pas etre reordonnees apres les
//! deux lectures : au moins l'un des deux voit l'autre.
//!
//!   * le dormeur voit la nouvelle generation -> il annule son parking ;
//!   * sinon le reveilleur voit `waiters > 0` **et** l'etat `Blocked`, donc sa
//!     transition reussit.
//!
//! Aucun des deux chemins ne perd le reveil. C'est cet argument que
//! `tools/smp/test_rendezvous.rs` met a l'epreuve avec de vrais fils.
//!
//! L'ordre sequentiel n'est pas un exces de prudence : c'est exactement ce que
//! le motif exige. `Release`/`Acquire` ordonnent une ecriture avec la lecture
//! QUI LA SUIT chez l'autre ; ils ne disent rien de deux ecritures suivies de
//! deux lectures croisees, qui est le cas ici.

use core::sync::atomic::{AtomicU64, Ordering};

/// Etat d'un dormeur, du seul point de vue du protocole.
///
/// Le noyau y branche l'etat d'ordonnancement de la tache ; le test y branche
/// un compteur. Les deux executent le MEME code.
pub trait Dormeur {
    /// Publie « je me gare ». Doit etre une ecriture d'ordre sequentiel.
    fn publie_parking(&self);
    /// Tente `gare -> reveille`. Rend vrai si CET appelant a gagne.
    fn tente_reveil(&self) -> bool;
    /// Defait une publication qui n'a plus lieu d'etre.
    fn annule_parking(&self);
}

/// Le point de rendez-vous : une generation, et un compte de dormeurs.
pub struct Rendezvous {
    generation: AtomicU64,
    dormeurs: AtomicU64,
}

impl Rendezvous {
    pub const fn neuf() -> Self {
        // La generation demarre a 1 : zero servirait de « jamais vu », et un
        // ticket nul se confondrait avec un ticket perime.
        Self {
            generation: AtomicU64::new(1),
            dormeurs: AtomicU64::new(0),
        }
    }

    /// Le ticket a presenter plus tard : l'etat observe maintenant.
    #[inline]
    pub fn ticket(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    #[inline]
    pub fn inscrit(&self) {
        self.dormeurs.fetch_add(1, Ordering::SeqCst);
    }

    #[inline]
    pub fn desinscrit(&self) {
        self.dormeurs.fetch_sub(1, Ordering::SeqCst);
    }

    #[inline]
    pub fn dormeurs(&self) -> u64 {
        self.dormeurs.load(Ordering::SeqCst)
    }

    /// Publie le parking, puis verifie qu'il a encore lieu d'etre.
    ///
    /// Rend `true` s'il faut reellement dormir, `false` si un reveil est passe
    /// entre-temps -- auquel cas le parking a deja ete annule.
    ///
    /// L'appelant doit s'etre inscrit AVANT d'appeler : c'est cette inscription
    /// que le reveilleur observe.
    pub fn doit_dormir(&self, ticket: u64, dormeur: &impl Dormeur) -> bool {
        dormeur.publie_parking();
        if self.generation.load(Ordering::SeqCst) != ticket {
            dormeur.annule_parking();
            return false;
        }
        true
    }

    /// Fait avancer la generation, sans reveiller personne.
    ///
    /// Le noyau parcourt lui-meme le registre des taches pour trouver ses
    /// dormeurs -- il ne les tient pas dans la file. Il lui faut donc les deux
    /// moities separement, mais DANS CET ORDRE : la generation d'abord, la
    /// lecture du compte ensuite. L'inverser laisserait valide le ticket d'un
    /// dormeur sur le point de s'inscrire, qui s'endormirait alors sur un
    /// reveil deja passe.
    #[inline]
    pub fn signale_seul(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Signale la file, puis reveille au plus `limite` dormeurs.
    ///
    /// La generation monte AVANT la lecture du compte : c'est l'autre moitie du
    /// motif croise. L'inverser rendrait la course a nouveau possible.
    pub fn signale<'a, D: Dormeur + 'a>(
        &self,
        limite: usize,
        candidats: impl Iterator<Item = &'a D>,
    ) -> usize {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if self.dormeurs.load(Ordering::SeqCst) == 0 {
            return 0;
        }
        let mut reveilles = 0;
        for candidat in candidats {
            if reveilles >= limite {
                break;
            }
            if candidat.tente_reveil() {
                reveilles += 1;
            }
        }
        reveilles
    }
}

impl Default for Rendezvous {
    fn default() -> Self {
        Self::neuf()
    }
}
