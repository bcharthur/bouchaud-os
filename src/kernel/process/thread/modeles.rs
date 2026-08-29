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

/// Contexte noyau sauvegarde lors d'un changement de tache.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Context {
    /// Sommet de pile noyau sauvegarde (tout le reste y est empile).
    pub rsp: u64,
}

