//! La POLITIQUE de reveil : ou poser une tache prete, et faut-il interrompre.
//!
//! # Le defaut que ce module ferme, et sa mesure
//!
//! Releve physique TRIGKEY, seize coeurs, Ladybird en charge :
//!
//! ```text
//! hid_poll_gap_max_ms     = 6783
//! hid_wake_to_run_max_us  = 6782927   <-- 99,99 % de l'ecart
//! hid_run_to_lock_max_us  = 2
//! hid_poll_body_max_us    = 162
//! ```
//!
//! Le fil `usb-hid` est une tache NOYAU de classe `Interactive`. Une fois
//! elue, elle prend son verrou en deux microsecondes et fait son travail en
//! cent soixante-deux. Elle a simplement attendu 6,78 s d'etre elue, pendant
//! que les quantums progressaient sur les seize coeurs.
//!
//! La lecture du chemin de reveil donne la cause exacte, et elle n'est pas
//! une lenteur : c'est un ENSEMBLE DE TROIS EXCLUSIONS qui se referment
//! toutes sur le meme predicat -- « la tache courante est-elle une tache
//! noyau ? » -- plus deux absences de secours.
//!
//!   1. `publish_ready` n'envoie l'IPI de replanification QUE si le coeur
//!      cible dort. Un coeur qui travaille ne recoit qu'un `need_resched`
//!      differe ;
//!   2. `reschedule_interrupt_handler` -- quand l'IPI arrive tout de meme --
//!      ne fait RIEN si la tache interrompue est une tache noyau : ni
//!      commutation, ni meme demande differee ;
//!   3. `running_user_cpu_mask` exclut du balayage de quantum les coeurs qui
//!      executent une tache noyau : ils ne recoivent jamais l'IPI periodique ;
//!   4. `preempt::safe_point` refuse de commuter si la tache courante est une
//!      tache noyau -- et aucun fil noyau n'appelle de toute facon de point
//!      sur : les huit sites existants sont tous sur le chemin des appels
//!      systeme ;
//!   5. `pression_volable` ne compte que la bande NORMALE. Une tache
//!      interactive seule en attente derriere un occupant ne rend donc jamais
//!      son coeur « volable » : aucun coeur au repos ne vient la chercher.
//!
//! Une tache interactive publiee sur un coeur qui execute un fil noyau
//! n'avait donc, litteralement, AUCUN chemin vers le processeur, hormis que
//! l'occupant veuille bien appeler `schedule()` de lui-meme. 6,78 s est le
//! temps qu'il a mis.
//!
//! # Ce que ce module decide, et ce qu'il ne decide pas
//!
//! Il ne connait ni les taches, ni les coeurs, ni l'horloge : on lui donne un
//! etat, il rend un placement et une decision. C'est ce qui permet a un test
//! hote de contredire la politique en une milliseconde, sans demarrer le
//! systeme -- et c'est ainsi que les six cas de la campagne ci-contre sont
//! verifies.
//!
//! Il ne connait AUCUN NOM DE TACHE non plus. Le privilege se demande par une
//! propriete declaree -- `latency_sensitive` --, pas par une comparaison de
//! chaine : l'audio, le compositeur et l'entree la demanderont de la meme
//! facon.

/// Temps minimal qu'un occupant SENSIBLE garde le processeur avant qu'un pair
/// sensible puisse le couper.
///
/// C'est la seule chose qui separe « faible latence » de « ping-pong ». Deux
/// fils sensibles qui se reveillent en meme temps se prendraient sinon le
/// coeur a chaque publication, et passeraient leur budget en commutations.
///
/// Un occupant NON sensible n'est pas protege par cette borne : il est
/// protege par la sienne, celle de son quantum, et ceder une milliseconde a
/// une tache qui en consomme cent soixante microsecondes ne lui coute rien de
/// mesurable.
pub const RESIDENCE_MINIMALE_NS: u64 = 1_000_000;

/// Budget d'activation d'une tache sensible a la latence.
///
/// FAIBLE BUDGET, FORTE EXIGENCE DE REVEIL -- et non priorite haute
/// permanente. Une tache qui consomme plus que cela dans une activation n'est
/// plus un fil d'entree periodique : c'est du calcul, et du calcul ne coupe
/// personne. Elle perd son privilege jusqu'a ce qu'une activation revienne
/// sous la borne.
///
/// Quatre millisecondes, soit un quantum. Le releve physique donne 162 us
/// pour une scrutation HID : deux ordres de grandeur de marge, et une tache
/// qui derive se desarme toute seule.
pub const BUDGET_ACTIVATION_NS: u64 = 4_000_000;

/// Classe d'ordonnancement, vue par la politique de reveil.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Classe {
    Interactive,
    Normale,
}

impl Default for Classe {
    fn default() -> Self {
        Self::Normale
    }
}

impl Classe {
    /// Plus le rang est bas, plus la classe passe tot.
    #[inline]
    pub const fn rang(self) -> u8 {
        match self {
            Self::Interactive => 0,
            Self::Normale => 1,
        }
    }
}

/// L'etat d'un coeur, tel que l'appelant le lit dans des atomiques.
#[derive(Clone, Copy, Debug, Default)]
pub struct Coeur {
    /// Ce coeur participe-t-il a l'ordonnancement ?
    pub en_ligne: bool,
    /// L'affinite de la tache reveillee l'autorise-t-elle ici ?
    pub autorise: bool,
    /// Ce coeur dort-il ? Lu APRES la mise en file et APRES la barriere.
    pub inactif: bool,
    /// L'occupant est-il lui-meme sensible a la latence ?
    pub occupant_sensible: bool,
    /// La classe de l'occupant.
    pub occupant_classe: Classe,
    /// Depuis combien de temps l'occupant tient-il le coeur ?
    pub residence_ns: u64,
    /// Taches interactives EN ATTENTE dans la file de ce coeur.
    pub attente_interactive: usize,
    /// Taches normales EN ATTENTE dans la file de ce coeur.
    pub attente_normale: usize,
}

/// La tache qu'on reveille.
#[derive(Clone, Copy, Debug)]
pub struct Reveille {
    pub classe: Classe,
    /// La PROPRIETE declaree, pas un nom : `latency_sensitive`.
    pub sensible: bool,
    /// Temps processeur consomme pendant l'activation precedente.
    pub consomme_ns: u64,
}

impl Reveille {
    /// Le privilege de reveil est-il actif pour cette activation ?
    ///
    /// TROIS conditions, et aucune n'est superflue :
    ///
    ///   * la PROPRIETE est declaree -- `latency_sensitive`, pas un nom ;
    ///   * la classe est `Interactive` -- une tache de fond declaree sensible
    ///     est une erreur de configuration, et une erreur de configuration ne
    ///     doit pas pouvoir couper le systeme ;
    ///   * l'activation precedente est restee SOUS SON BUDGET. Sans cela le
    ///     privilege serait une priorite permanente, exactement ce qu'il ne
    ///     doit pas etre.
    #[inline]
    pub fn privilegiee(&self) -> bool {
        self.sensible
            && self.classe.rang() == Classe::Interactive.rang()
            && self.consomme_ns < BUDGET_ACTIVATION_NS
    }
}

/// Ce que l'appelant doit faire APRES la mise en file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Decision {
    /// Le coeur dort : demande + IPI. Il se reveille et elit.
    ReveilImmediat,
    /// Le coeur travaille et doit ceder MAINTENANT : demande ciblee + IPI.
    /// C'est la seule decision qui autorise la preemption d'un fil noyau.
    PreemptionCiblee,
    /// Le coeur travaille : demande seule, servie a son prochain point sur ou
    /// a son prochain quantum.
    DemandeDifferee,
    /// Rien de plus : le coeur n'est pas adressable.
    MiseEnFile,
}

/// Le cout d'un coeur pour une tache sensible. Plus bas vaut mieux.
///
/// L'ordre est lexicographique et TOTAL : un coeur au repos bat tout coeur
/// occupe ; a occupation egale, celui qui fait le moins attendre ; a attente
/// egale, le coeur precedent -- la residence de cache est gratuite ; et enfin
/// l'indice, pour que la decision soit deterministe et donc testable.
fn cout(coeur: &Coeur, indice: usize, precedent: usize) -> (u8, usize, u8, usize) {
    // Quatre rangs d'occupation, du moins couteux a deplacer au plus couteux :
    // un coeur au repos ne coute rien ; une tache de fond coute peu ; une
    // interactive coute une reponse ; un pair sensible coute SA latence, et
    // c'est celle-la qu'on protege en dernier.
    let rang = if coeur.inactif {
        0
    } else if coeur.occupant_sensible {
        3
    } else {
        1 + (1 - coeur.occupant_classe.rang())
    };
    // Une interactive en attente pese plus qu'une normale : c'est elle qui
    // passera devant nous.
    let attente = coeur
        .attente_interactive
        .saturating_mul(4)
        .saturating_add(coeur.attente_normale);
    let affinite = if indice == precedent { 0 } else { 1 };
    (rang, attente, affinite, indice)
}

/// Choisit le coeur d'accueil d'une tache qu'on reveille.
///
/// Rend `None` quand la politique n'a rien a dire -- l'appelant garde alors
/// son choix historique. C'est volontaire : ce module ne remplace pas le
/// placement general, il ajoute celui des taches sensibles a la latence.
///
/// # Pourquoi le placement vient AVANT la preemption
///
/// Preempter coute une commutation a quelqu'un. Poser la tache sur un coeur
/// qui dort ne coute rien a personne et reveille par un IPI, chemin deja
/// eprouve et deja protege contre le reveil perdu. Sur une machine a seize
/// coeurs dont un ou deux travaillent, c'est le cas NORMAL, et c'est lui qui
/// ramene 6,78 s a quelques microsecondes.
pub fn choisit_coeur(precedent: usize, reveille: &Reveille, coeurs: &[Coeur]) -> Option<usize> {
    let mut selection = Selection::neuve(precedent, reveille.privilegiee());
    for (indice, coeur) in coeurs.iter().enumerate() {
        selection.propose(indice, coeur);
    }
    selection.retenu()
}

/// Le meme choix, EN FLOT : un coeur a la fois, sans tableau.
///
/// `publish_ready` s'execute aussi depuis une IRQ. Y poser un tableau de
/// seize etats de coeur -- six cents octets -- sur la pile d'une continuation
/// d'interruption serait un cout impose a tous les reveils pour le confort
/// d'une boucle. Le choix est un minimum : il se calcule en un seul passage,
/// sans rien retenir d'autre que le meilleur.
///
/// `choisit_coeur` n'est que cette structure derriere une tranche, et c'est
/// elle que les tests hote mettent a l'epreuve : les deux chemins ne peuvent
/// pas diverger.
pub struct Selection {
    precedent: usize,
    actif: bool,
    meilleur: Option<(usize, (u8, usize, u8, usize))>,
}

impl Selection {
    /// `actif` est `reveille.privilegiee()`. Faux, la selection ne retient
    /// rien : l'appelant garde son placement historique.
    #[inline]
    pub const fn neuve(precedent: usize, actif: bool) -> Self {
        Self { precedent, actif, meilleur: None }
    }

    #[inline]
    pub fn propose(&mut self, indice: usize, coeur: &Coeur) {
        if !self.actif || !coeur.en_ligne || !coeur.autorise {
            return;
        }
        let cout = cout(coeur, indice, self.precedent);
        if self.meilleur.map(|(_, deja)| cout < deja).unwrap_or(true) {
            self.meilleur = Some((indice, cout));
        }
    }

    #[inline]
    pub fn retenu(&self) -> Option<usize> {
        self.meilleur.map(|(indice, _)| indice)
    }
}

/// Que faire, une fois la tache mise en file sur `cible` ?
pub fn decide(cible: &Coeur, reveille: &Reveille) -> Decision {
    if !cible.en_ligne {
        return Decision::MiseEnFile;
    }
    if cible.inactif {
        return Decision::ReveilImmediat;
    }
    if reveille.privilegiee() {
        // UN PAIR SENSIBLE GARDE SA TRANCHE MINIMALE.
        //
        // Sans cette borne, deux fils sensibles se coupent mutuellement a
        // chaque publication et ne font plus que commuter.
        if cible.occupant_sensible && cible.residence_ns < RESIDENCE_MINIMALE_NS {
            return Decision::DemandeDifferee;
        }
        return Decision::PreemptionCiblee;
    }
    // Le comportement historique, inchange : mise en file et demande differee.
    // Une tache normale ne coupe personne, et une interactive non sensible
    // attend le prochain point sur ou le prochain quantum -- ce qui FONCTIONNE
    // pour une tache utilisateur, seule categorie a laquelle ces deux
    // mecanismes s'appliquent.
    Decision::DemandeDifferee
}

/// Le coeur le plus charge en taches EN ATTENTE, pour le secours par vol.
///
/// `pression_volable` ne comptait que la bande normale, au motif qu'une tache
/// interactive volee paie une migration « au moment precis ou elle doit
/// repondre ». Le raisonnement oublie le cas ou elle ne repond PAS : une
/// interactive en attente derriere un fil noyau ne paie pas une migration,
/// elle paie six secondes. Un cache froid vaut mieux que cela.
///
/// La preference reste au travail de fond -- `FileCpu::vole` sert la bande
/// normale en premier --, mais la PRESSION compte desormais les deux bandes,
/// sinon le coeur n'est jamais candidat et l'ordre de service ne sert a rien.
#[inline]
pub fn pression_de_secours(attente_interactive: usize, attente_normale: usize) -> usize {
    attente_interactive.saturating_add(attente_normale)
}
