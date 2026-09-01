/// Taille de la pile noyau d'une tache (64 KiB).
const KSTACK_SIZE: usize = 64 * 1024;

/// Classe d'ordonnancement d'une tache.
///
/// Deux, pas davantage. L'audit OS avait identifie l'absence de priorites comme
/// le dernier manque avant un processus de rendu separe : sur un cœur unique et
/// un tourniquet strict, sortir le rendu d'un processus n'empeche pas une page
/// lourde de rendre l'interface lente, parce que rien ne favorise l'interface.
///
/// Ce qu'il fallait n'etait pas un ordonnanceur different — le tourniquet
/// convient — mais un moyen de dire lequel des deux compte quand les deux sont
/// prets. Deux classes suffisent a le dire, et une troisieme n'ajouterait que
/// des questions sans reponse.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Priorite {
    /// Ce qui repond a l'utilisateur : l'interface du navigateur, le serveur
    /// graphique. Servi en premier quand plusieurs taches sont pretes.
    Interactive,
    /// Tout le reste : calcul, rendu, travail de fond. Jamais affame.
    Normale,
}

impl Priorite {
    #[inline]
    pub const fn code(self) -> u8 {
        match self {
            Self::Interactive => 0,
            Self::Normale => 1,
        }
    }

    #[inline]
    pub const fn depuis_code(code: u8) -> Self {
        // Un code inconnu ne peut venir que d'une ecriture corrompue.
        // `Normale` est le choix sur : une tache trop prioritaire affamerait
        // le reste, une tache normale de trop ne fait que passer apres.
        match code {
            0 => Self::Interactive,
            _ => Self::Normale,
        }
    }
}

// La priorite se lit depuis n'importe quel coeur -- l'election de la
// prochaine tache la consulte -- et s'ecrit depuis un autre, quand
// `setpriority` la change pour tout un processus. C'est le meme motif que
// l'etat d'ordonnancement, et elle devient atomique pour la meme raison :
// sans cela, la changer demanderait un acces exclusif a la tache, donc de
// serialiser une simple ecriture d'octet.
#[repr(transparent)]
pub struct PrioriteAtomique(core::sync::atomic::AtomicU8);

impl PrioriteAtomique {
    #[inline]
    pub const fn neuve(priorite: Priorite) -> Self {
        Self(core::sync::atomic::AtomicU8::new(priorite.code()))
    }
    #[inline]
    pub fn charge(&self) -> Priorite {
        Priorite::depuis_code(self.0.load(core::sync::atomic::Ordering::Acquire))
    }
    #[inline]
    pub fn range(&self, priorite: Priorite) {
        self.0.store(priorite.code(), core::sync::atomic::Ordering::Release);
    }
}

impl PartialEq<Priorite> for PrioriteAtomique {
    #[inline]
    fn eq(&self, autre: &Priorite) -> bool {
        self.charge() == *autre
    }
}

impl core::fmt::Debug for PrioriteAtomique {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(&self.charge(), f)
    }
}

/// Nombre maximal de tours consecutifs accordes aux taches interactives.
///
/// Sans cette borne, une tache interactive qui calcule sans jamais se bloquer
/// affamerait tout le reste — et « l'interface reste fluide » deviendrait
/// « rien d'autre ne tourne ». Au-dela du compte, le tourniquet reprend ses
/// droits pour un tour, ce qui garantit une progression a toute tache prete.
///
/// Quatre : l'interface conserve des rafales courtes, mais une tache normale
/// recupere au moins un tour sur cinq sous pression interactive continue.
/// C'est volontairement plus favorable a WebContent et aux workers CPU.
const TOURS_INTERACTIFS_MAX: u32 = 4;


/// Etat d'ordonnancement d'une tache.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TaskState {
    /// Prete a s'executer (ou en cours).
    Ready,
    /// En attente d'un evenement (futex, sommeil, entree).
    Blocked,
    /// Terminee, en attente de nettoyage.
    Zombie,
}

impl TaskState {
    #[inline]
    pub const fn code(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::Blocked => 1,
            Self::Zombie => 2,
        }
    }

    #[inline]
    pub const fn depuis_code(code: u8) -> Self {
        match code {
            1 => Self::Blocked,
            2 => Self::Zombie,
            // Un code inconnu ne peut venir que d'une ecriture corrompue.
            // `Ready` est le choix sur : une tache prete de trop est
            // ordonnancee pour rien, une tache bloquee de trop ne repart
            // jamais.
            _ => Self::Ready,
        }
    }
}

// BOUCHAUD_C1_ETAT_TACHE_ATOMIQUE_V1
//
// POURQUOI L'ETAT DEVIENT ATOMIQUE
// --------------------------------
// `state` etait un simple champ d'enumeration. Le lire ou l'ecrire depuis un
// autre CPU que celui qui execute la tache n'etait sur que parce que le gros
// verrou serialisait TOUT le noyau -- et c'est precisement ce qu'on retire.
//
// Or ce champ est, par nature, celui que deux coeurs se disputent : un CPU
// bloque sa tache pendant qu'un autre la reveille. C'est la course centrale de
// n'importe quel ordonnanceur SMP.
//
// En atomique, l'ecriture d'un CPU est visible par les autres sans aucun
// verrou. Le gros verrou n'est plus ce qui rend `state` sur ; il ne protege
// plus que la STRUCTURE de la table, ce qui est un tout autre besoin et une
// section critique beaucoup plus courte.
//
// L'egalite est implementee contre `TaskState` pour que les soixante-cinq
// lectures existantes -- `task.state == TaskState::Ready` -- restent
// inchangees. Seules les ECRITURES deviennent explicites (`range`), ce qui est
// souhaitable : une transition d'etat merite d'etre visible a la lecture.
#[repr(transparent)]
pub struct EtatAtomique(core::sync::atomic::AtomicU8);

impl EtatAtomique {
    #[inline]
    pub const fn neuf(etat: TaskState) -> Self {
        Self(core::sync::atomic::AtomicU8::new(etat.code()))
    }

    #[inline]
    pub fn charge(&self) -> TaskState {
        TaskState::depuis_code(self.0.load(core::sync::atomic::Ordering::Acquire))
    }

    /// Publie un nouvel etat.
    ///
    /// # Pourquoi `SeqCst` et non `Release`
    ///
    /// `Release` suffirait a rendre visibles les ecritures precedentes -- une
    /// tache reveillee ne doit pas repartir sur une echeance perimee. Il ne
    /// suffit PAS au protocole de reveil perdu, qui est un motif
    /// ecriture-puis-lecture CROISE :
    ///
    ///   le dormeur  ecrit `Blocked`, puis lit la generation ;
    ///   le reveilleur ecrit la generation, puis lit l'etat.
    ///
    /// Sans ordre total, les deux lectures peuvent remonter avant les deux
    /// ecritures : le dormeur ne voit pas le reveil, le reveilleur ne voit pas
    /// le dormeur, et la tache ne repart jamais. Sur x86 cela coute une
    /// instruction verrouillee -- au prix d'un changement d'etat, pas d'une
    /// boucle chaude.
    #[inline]
    pub fn range(&self, etat: TaskState) {
        self.0.store(etat.code(), core::sync::atomic::Ordering::SeqCst);
    }

    /// Transition CONDITIONNELLE : ne passe a `nouveau` que si l'etat vaut
    /// encore `attendu`.
    ///
    /// C'est ce qui remplace « lire, decider, ecrire » sous gros verrou. Deux
    /// CPU qui reveillent la meme tache bloquee doivent en avoir exactement un
    /// qui gagne ; sans cela, elle serait mise deux fois en file d'execution.
    #[inline]
    pub fn echange(&self, attendu: TaskState, nouveau: TaskState) -> bool {
        self.0
            .compare_exchange(
                attendu.code(),
                nouveau.code(),
                core::sync::atomic::Ordering::AcqRel,
                core::sync::atomic::Ordering::Acquire,
            )
            .is_ok()
    }
}

impl PartialEq<TaskState> for EtatAtomique {
    #[inline]
    fn eq(&self, autre: &TaskState) -> bool {
        self.charge() == *autre
    }
}

impl core::fmt::Debug for EtatAtomique {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(&self.charge(), f)
    }
}

/// Contexte noyau sauvegarde lors d'un changement de tache.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Context {
    /// Sommet de pile noyau sauvegarde (tout le reste y est empile).
    pub rsp: u64,
}

// BOUCHAUD_C1_CHAMPS_TACHE_ATOMIQUES_V1
//
// Les autres champs que DEUX CPU se disputent.
//
// L'etat n'est pas seul en cause. Reveiller une tache, c'est aussi lire son
// echeance, sa cle de file d'attente, sa cle de futex -- depuis un CPU qui
// n'est pas le sien. Les laisser en champs ordinaires reviendrait a rendre
// `state` atomique et a garder le gros verrou pour tout le reste : le
// sous-systeme n'aurait pas bouge.
//
// Chacun garde le nom du champ d'origine ; seules les ECRITURES deviennent
// explicites. C'est voulu : une ecriture visible d'un autre coeur merite
// d'etre visible a la lecture du code.

macro_rules! champ_atomique {
    ($nom:ident, $atomique:ty, $valeur:ty) => {
        #[repr(transparent)]
        pub struct $nom($atomique);

        impl $nom {
            #[inline]
            pub const fn neuf(valeur: $valeur) -> Self {
                Self(<$atomique>::new(valeur))
            }
            #[inline]
            pub fn charge(&self) -> $valeur {
                self.0.load(core::sync::atomic::Ordering::Acquire)
            }
            #[inline]
            pub fn range(&self, valeur: $valeur) {
                self.0.store(valeur, core::sync::atomic::Ordering::Release)
            }
            /// Lit et remplace d'un seul coup.
            ///
            /// Sert aux consommations uniques : celui qui recupere la valeur
            /// doit etre le seul, sinon deux CPU serviraient la meme echeance.
            #[inline]
            pub fn echange(&self, valeur: $valeur) -> $valeur {
                self.0.swap(valeur, core::sync::atomic::Ordering::AcqRel)
            }
        }

        impl PartialEq<$valeur> for $nom {
            #[inline]
            fn eq(&self, autre: &$valeur) -> bool {
                self.charge() == *autre
            }
        }

        impl PartialOrd<$valeur> for $nom {
            #[inline]
            fn partial_cmp(&self, autre: &$valeur) -> Option<core::cmp::Ordering> {
                self.charge().partial_cmp(autre)
            }
        }

        impl core::fmt::Debug for $nom {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                core::fmt::Debug::fmt(&self.charge(), f)
            }
        }

        // Ces champs apparaissent tels quels dans les journaux de diagnostic.
        // Sans `Display`, chaque site d'affichage devrait appeler `charge()`,
        // ce qui alourdirait des lignes de trace deja denses.
        impl core::fmt::Display for $nom {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                core::fmt::Display::fmt(&self.charge(), f)
            }
        }
    };
}

champ_atomique!(EcheanceAtomique, core::sync::atomic::AtomicU64, u64);
champ_atomique!(CleAtomique, core::sync::atomic::AtomicUsize, usize);
champ_atomique!(DrapeauAtomique, core::sync::atomic::AtomicBool, bool);
champ_atomique!(CoeurAtomique, core::sync::atomic::AtomicU8, u8);
champ_atomique!(CoeurSigneAtomique, core::sync::atomic::AtomicI8, i8);
