//! LES FAUTES DE PAGE, PAR PROCESSUS ET PAR CATEGORIE, AVEC LEUR COUT.
//!
//! # Le defaut, mesure sur la TRIGKEY
//!
//! Le journal physique du navigateur porte ceci :
//!
//! ```text
//! PERF-BROWSER pid=5 frames_delta=61 inputs_delta=0 bottleneck=memory-pagefault
//! ```
//!
//! « memory-pagefault » est un verdict, pas une mesure. Il ne dit ni combien
//! de fautes, ni de quelle sorte, ni combien de temps elles ont coute, ni
//! lequel des six processus Ladybird les prend. Or ces quatre questions ont
//! quatre remedes differents :
//!
//! ```text
//!   des fautes Zero en masse      -> une pile ou un tas qui grandit
//!   des fautes FichierPrive       -> le chargement paresseux d'un binaire
//!   des fautes Partage            -> une surface projetee, un cache de pages
//!   des fautes Copie              -> un fork, ou une page propre ecrite
//! ```
//!
//! Le noyau comptait deja des fautes. Mais il les comptait PAR PROCESSEUR
//! (`STALL_PF_BEGIN[cpu]`), et globalement par sorte (`FAULTS_ZERO`,
//! `FAULTS_FILE`). Sur une machine a seize coeurs qui fait tourner six
//! processus Ladybird, aucune de ces deux vues ne repond a la question
//! « lequel, et combien cela lui coute ».
//!
//! # Ce que ce fichier decide
//!
//! Il tient un livre de comptes borne : pour chaque processus observe, un
//! compte par categorie -- combien de fautes, combien de nanosecondes au
//! total, et la pire d'entre elles.
//!
//! La pire compte autant que le total. Mille fautes a dix microsecondes et
//! dix fautes a une milliseconde donnent le meme total ; la premiere est le
//! fonctionnement normal d'un chargement paresseux, la seconde est une
//! saccade que l'utilisateur voit.
//!
//! # Pourquoi ce livre est BORNE, et ce que la borne coute
//!
//! Un livre qui grandit avec le nombre de processus vus depuis le demarrage
//! est un fuite de memoire a horizon lent, et il se consulte en temps
//! proportionnel a son age. Le livre a donc un nombre fixe d'entrees et
//! chasse, quand il est plein, celle dont la derniere faute est la plus
//! ancienne.
//!
//! Ce que cette borne coute est precis et il faut le dire : un processus
//! chasse perd son historique, et s'il refaute il repart de zero. C'est le
//! bon compromis ici, parce que la question posee -- « qui paie des fautes en
//! ce moment » -- porte sur le present. Un processus qui n'a pas faute depuis
//! longtemps n'est, par construction, pas la reponse.
//!
//! # Pourquoi ce fichier ne depend de rien
//!
//! Ni `crate::`, ni `unsafe`, ni horloge : l'instant est un PARAMETRE. Le
//! livre se compile donc sur l'hote et `tools/process/test_fautes.rs`
//! l'exerce a chaque CI, alors que le chemin de faute de page ne s'execute
//! que dans QEMU -- et que ses erreurs (un compte attribue au mauvais
//! processus, une entree chassee qui emporte la mauvaise) sont exactement le
//! genre de faute qu'aucun test d'integration ne remarque.

/// Les sortes de faute que le noyau sait distinguer a la resolution.
///
/// Elles ne sont pas inventees : chacune correspond a une branche reelle de
/// `peuple_a_la_demande`, c'est-a-dire a un `PromesseBacking` et a un
/// `ResidentKind`. Une categorie de plus ici sans branche correspondante
/// serait une colonne qui reste vide pour toujours.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Categorie {
    /// `PromesseBacking::Zero` : pile, tas, `mmap` anonyme.
    Zero,
    /// `PromesseBacking::File` : le chargement paresseux d'un ELF ou d'un
    /// fichier projete en prive.
    FichierPrive,
    /// `PromesseBacking::SharedFile` : une surface `MAP_SHARED`, le cache de
    /// pages partage entre deux processus.
    Partage,
    /// `PromesseBacking::Framebuffer` : de la memoire de peripherique.
    Materiel,
    /// Une page propre devenue ecrivable : copie sur ecriture.
    Copie,
    /// La page etait deja en cours de chargement par un AUTRE processeur :
    /// cette faute-ci n'a rien charge, elle a attendu.
    ///
    /// Elle merite sa propre colonne parce qu'elle ne se soigne pas comme les
    /// autres. Les cinq precedentes se reduisent en faisant moins de travail ;
    /// celle-ci se reduit en faisant moins de CONTENTION -- et sur un portage
    /// ou six processus Ladybird partagent des pages, c'est une mesure a part
    /// entiere, pas un detail de comptabilite.
    Attente,
    /// La faute n'a pas ete resolue.
    Echec,
}

pub const CATEGORIES: usize = 7;

impl Categorie {
    pub fn rang(self) -> usize {
        match self {
            Categorie::Zero => 0,
            Categorie::FichierPrive => 1,
            Categorie::Partage => 2,
            Categorie::Materiel => 3,
            Categorie::Copie => 4,
            Categorie::Attente => 5,
            Categorie::Echec => 6,
        }
    }

    pub fn depuis_rang(rang: usize) -> Option<Categorie> {
        match rang {
            0 => Some(Categorie::Zero),
            1 => Some(Categorie::FichierPrive),
            2 => Some(Categorie::Partage),
            3 => Some(Categorie::Materiel),
            4 => Some(Categorie::Copie),
            5 => Some(Categorie::Attente),
            6 => Some(Categorie::Echec),
            _ => None,
        }
    }

    /// Le nom court, pour une colonne de tableau.
    pub fn nom(self) -> &'static str {
        match self {
            Categorie::Zero => "zero",
            Categorie::FichierPrive => "fichier",
            Categorie::Partage => "partage",
            Categorie::Materiel => "materiel",
            Categorie::Copie => "copie",
            Categorie::Attente => "attente",
            Categorie::Echec => "echec",
        }
    }
}

/// Ce qu'on retient d'une categorie pour un processus.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Compte {
    pub nombre: u64,
    pub total_ns: u64,
    /// La plus longue faute observee. Voir l'en-tete : elle ne se deduit pas
    /// du total.
    pub pire_ns: u64,
}

impl Compte {
    pub fn vide(&self) -> bool {
        self.nombre == 0
    }

    /// La duree moyenne, ou zero si rien n'a ete observe.
    ///
    /// Rendre zero plutot qu'une division par zero est un choix : une moyenne
    /// sur zero echantillon n'existe pas, et l'appelant qui l'affiche doit
    /// regarder `nombre` avant. `vide()` est la pour cela.
    pub fn moyenne_ns(&self) -> u64 {
        if self.nombre == 0 {
            return 0;
        }
        self.total_ns / self.nombre
    }

    pub fn ajoute(&mut self, duree_ns: u64) {
        self.nombre = self.nombre.saturating_add(1);
        self.total_ns = self.total_ns.saturating_add(duree_ns);
        if duree_ns > self.pire_ns {
            self.pire_ns = duree_ns;
        }
    }

    pub fn fusionne(&mut self, autre: &Compte) {
        self.nombre = self.nombre.saturating_add(autre.nombre);
        self.total_ns = self.total_ns.saturating_add(autre.total_ns);
        if autre.pire_ns > self.pire_ns {
            self.pire_ns = autre.pire_ns;
        }
    }
}

/// La DECOMPOSITION d'une faute FichierPrive, pour UN processus.
///
/// BOUCHAUD_C54_PAR_PID_OU_RIEN
///
/// La premiere version publiait `FAULT_FILE_BREAKDOWN pid=18 ...` a partir
/// d'atomiques GLOBAUX lus a la sortie du processus 18. L'etiquette promettait
/// une attribution que les chiffres n'avaient pas : quand WebContent, le
/// Compositor et le worker travaillent en meme temps, le « cout du worker »
/// contenait celui des deux autres.
///
/// La difference entre deux sorties successives ne repare rien : elle suppose
/// que les processus ne se chevauchent pas, ce qui est precisement faux dans
/// le cas qu'on veut mesurer.
///
/// # Ce qui est ADDITIF et ce qui ne l'est pas
///
/// `acquire_ns` CONTIENT `acquire_backing_ns` : l'acquisition dans le cache de
/// pages fait la lecture du support elle-meme. Les additionner compterait la
/// lecture deux fois. `acquire_backing_ns` est donc un « DONT », explicatif.
///
/// De meme `hit_ns + miss_ns + wait_ns == acquire_ns` : ce sont les trois
/// issues possibles d'une acquisition, pas trois phases successives.
///
/// La somme qui doit approcher `total_ns` est :
///
///     attente + acquire + backing_direct + mm + map
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct PhasesFichier {
    pub nombre: u64,
    pub total_ns: u64,
    /// Attente qu'un AUTRE coeur finisse de charger la meme page.
    pub attente_ns: u64,
    /// Temps total dans l'acquisition du cache de pages propres.
    pub acquire_ns: u64,
    /// DONT la lecture du support faite a l'interieur. Non additif.
    pub acquire_backing_ns: u64,
    pub hit_n: u64,
    pub hit_ns: u64,
    pub miss_n: u64,
    pub miss_ns: u64,
    pub wait_n: u64,
    pub wait_ns: u64,
    /// Lecture du support faite HORS acquisition (chemin de construction).
    pub backing_direct_ns: u64,
    pub mm_ns: u64,
    pub map_ns: u64,
    pub pire_ns: u64,
}

impl PhasesFichier {
    /// Ce que la decomposition explique. Voir l'en-tete : `acquire_backing`
    /// n'y figure PAS, il est deja dans `acquire`.
    pub fn explique_ns(&self) -> u64 {
        self.attente_ns
            .saturating_add(self.acquire_ns)
            .saturating_add(self.backing_direct_ns)
            .saturating_add(self.mm_ns)
            .saturating_add(self.map_ns)
    }

    /// Ce qu'elle n'explique pas. Publie, jamais reparti.
    pub fn residu_ns(&self) -> u64 {
        self.total_ns.saturating_sub(self.explique_ns())
    }

    pub fn residu_pct(&self) -> u64 {
        if self.total_ns == 0 {
            return 0;
        }
        self.residu_ns().saturating_mul(100) / self.total_ns
    }

    pub fn ajoute(&mut self, autre: &PhasesFichier) {
        self.nombre = self.nombre.saturating_add(autre.nombre);
        self.total_ns = self.total_ns.saturating_add(autre.total_ns);
        self.attente_ns = self.attente_ns.saturating_add(autre.attente_ns);
        self.acquire_ns = self.acquire_ns.saturating_add(autre.acquire_ns);
        self.acquire_backing_ns =
            self.acquire_backing_ns.saturating_add(autre.acquire_backing_ns);
        self.hit_n = self.hit_n.saturating_add(autre.hit_n);
        self.hit_ns = self.hit_ns.saturating_add(autre.hit_ns);
        self.miss_n = self.miss_n.saturating_add(autre.miss_n);
        self.miss_ns = self.miss_ns.saturating_add(autre.miss_ns);
        self.wait_n = self.wait_n.saturating_add(autre.wait_n);
        self.wait_ns = self.wait_ns.saturating_add(autre.wait_ns);
        self.backing_direct_ns =
            self.backing_direct_ns.saturating_add(autre.backing_direct_ns);
        self.mm_ns = self.mm_ns.saturating_add(autre.mm_ns);
        self.map_ns = self.map_ns.saturating_add(autre.map_ns);
        if autre.pire_ns > self.pire_ns {
            self.pire_ns = autre.pire_ns;
        }
    }
}

/// Le nombre de processus suivis simultanement.
///
/// Seize : le portage fait tourner un BouchaudBrowserHost, un WebContent, un
/// RequestServer, un ImageDecoder, un Compositor et des WebWorker a la
/// demande, plus le bureau et le shell. Seize laisse la place a un second
/// navigateur sans chasser le premier.
pub const PROCESSUS_MAX: usize = 16;

#[derive(Clone, Copy)]
struct Entree {
    pid: u32,
    occupee: bool,
    derniere_ns: u64,
    comptes: [Compte; CATEGORIES],
    phases: PhasesFichier,
}

impl Entree {
    const fn libre() -> Entree {
        Entree {
            pid: 0,
            occupee: false,
            derniere_ns: 0,
            comptes: [Compte { nombre: 0, total_ns: 0, pire_ns: 0 }; CATEGORIES],
            phases: PhasesFichier {
                nombre: 0, total_ns: 0, attente_ns: 0, acquire_ns: 0,
                acquire_backing_ns: 0, hit_n: 0, hit_ns: 0, miss_n: 0,
                miss_ns: 0, wait_n: 0, wait_ns: 0, backing_direct_ns: 0,
                mm_ns: 0, map_ns: 0, pire_ns: 0,
            },
        }
    }
}

/// Le livre de comptes.
pub struct Journal {
    entrees: [Entree; PROCESSUS_MAX],
    /// Combien de processus ont ete chasses faute de place. Un compteur qui
    /// monte dit que la borne est trop basse pour cette machine -- et le dire
    /// vaut mieux que de rendre des chiffres silencieusement incomplets.
    pub chasses: u64,
}

impl Default for Journal {
    fn default() -> Self {
        Journal::neuf()
    }
}

impl Journal {
    pub const fn neuf() -> Journal {
        Journal { entrees: [Entree::libre(); PROCESSUS_MAX], chasses: 0 }
    }

    fn rang_de(&self, pid: u32) -> Option<usize> {
        for (rang, entree) in self.entrees.iter().enumerate() {
            if entree.occupee && entree.pid == pid {
                return Some(rang);
            }
        }
        None
    }

    /// Le rang ou ecrire pour ce processus, en chassant si necessaire.
    fn rang_pour(&mut self, pid: u32) -> usize {
        if let Some(rang) = self.rang_de(pid) {
            return rang;
        }
        for (rang, entree) in self.entrees.iter().enumerate() {
            if !entree.occupee {
                let rang = rang;
                self.entrees[rang] = Entree::libre();
                self.entrees[rang].pid = pid;
                self.entrees[rang].occupee = true;
                return rang;
            }
        }
        // Plein : on chasse la plus ancienne. `derniere_ns` et non le nombre
        // de fautes -- un processus tres actif il y a une minute puis endormi
        // n'interesse plus, alors qu'un processus qui vient de commencer a
        // fauter est precisement celui qu'on cherche.
        let mut victime = 0usize;
        for (rang, entree) in self.entrees.iter().enumerate() {
            if entree.derniere_ns < self.entrees[victime].derniere_ns {
                victime = rang;
            }
        }
        self.chasses = self.chasses.saturating_add(1);
        self.entrees[victime] = Entree::libre();
        self.entrees[victime].pid = pid;
        self.entrees[victime].occupee = true;
        victime
    }

    /// Enregistre une faute resolue (ou echouee) pour ce processus.
    pub fn note(&mut self, pid: u32, categorie: Categorie, duree_ns: u64, maintenant_ns: u64) {
        let rang = self.rang_pour(pid);
        let entree = &mut self.entrees[rang];
        // L'horloge peut reculer entre deux coeurs. Garder le maximum evite
        // qu'un processus vivant paraisse plus vieux que les autres et se
        // fasse chasser par sa propre mesure.
        if maintenant_ns > entree.derniere_ns {
            entree.derniere_ns = maintenant_ns;
        }
        entree.comptes[categorie.rang()].ajoute(duree_ns);
    }

    /// Enregistre la decomposition d'UNE faute FichierPrive pour CE processus.
    ///
    /// Le meme rang que `note` : la decomposition suit le processus, pas
    /// l'ordre des sorties.
    pub fn note_phases(&mut self, pid: u32, phases: &PhasesFichier, maintenant_ns: u64) {
        let rang = self.rang_pour(pid);
        let entree = &mut self.entrees[rang];
        if maintenant_ns > entree.derniere_ns {
            entree.derniere_ns = maintenant_ns;
        }
        entree.phases.ajoute(phases);
    }

    /// La decomposition accumulee de ce processus.
    pub fn phases(&self, pid: u32) -> PhasesFichier {
        match self.rang_de(pid) {
            Some(rang) => self.entrees[rang].phases,
            None => PhasesFichier::default(),
        }
    }

    pub fn compte(&self, pid: u32, categorie: Categorie) -> Compte {
        match self.rang_de(pid) {
            Some(rang) => self.entrees[rang].comptes[categorie.rang()],
            None => Compte::default(),
        }
    }

    /// Toutes categories confondues.
    pub fn total(&self, pid: u32) -> Compte {
        let mut total = Compte::default();
        let Some(rang) = self.rang_de(pid) else { return total };
        for compte in self.entrees[rang].comptes.iter() {
            total.fusionne(compte);
        }
        total
    }

    /// La categorie qui coute le plus de TEMPS a ce processus.
    ///
    /// Le temps, et non le nombre : c'est le temps que l'utilisateur ressent.
    /// Rend `None` si le processus n'a jamais faute.
    pub fn categorie_dominante(&self, pid: u32) -> Option<(Categorie, Compte)> {
        let rang = self.rang_de(pid)?;
        let mut meilleure: Option<(Categorie, Compte)> = None;
        for index in 0..CATEGORIES {
            let compte = self.entrees[rang].comptes[index];
            if compte.vide() {
                continue;
            }
            let Some(categorie) = Categorie::depuis_rang(index) else { continue };
            match meilleure {
                Some((_, actuelle)) if actuelle.total_ns >= compte.total_ns => {}
                _ => meilleure = Some((categorie, compte)),
            }
        }
        meilleure
    }

    /// La part de la fenetre passee en faute, en pour mille.
    ///
    /// EN POUR MILLE, et non en pourcent. Un processus qui perd trois
    /// millisecondes par seconde est a 0 % en pourcent entier : le chiffre
    /// qui compte disparaitrait dans l'arrondi, et c'est justement l'ordre de
    /// grandeur qu'on cherche a suivre quand on optimise.
    pub fn part_pour_mille(&self, pid: u32, fenetre_ns: u64) -> Option<u64> {
        if fenetre_ns == 0 {
            return None;
        }
        let total = self.total(pid);
        if total.vide() {
            return None;
        }
        Some(total.total_ns.saturating_mul(1000) / fenetre_ns)
    }

    /// Oublie ce processus. Appele a sa mort : un PID se reutilise, et
    /// heriter des comptes du precedent occupant serait pire que ne rien
    /// savoir.
    pub fn oublie(&mut self, pid: u32) {
        if let Some(rang) = self.rang_de(pid) {
            self.entrees[rang] = Entree::libre();
        }
    }

    pub fn suivis(&self) -> usize {
        self.entrees.iter().filter(|entree| entree.occupee).count()
    }

    /// Les processus suivis, du plus couteux en temps au moins couteux.
    ///
    /// Ecrit dans `sortie` et rend combien d'entrees ont ete ecrites : pas
    /// d'allocation, le noyau appelle ceci depuis un chemin de diagnostic.
    pub fn classement(&self, sortie: &mut [(u32, Compte)]) -> usize {
        let mut ecrits = 0usize;
        for entree in self.entrees.iter() {
            if !entree.occupee || ecrits >= sortie.len() {
                continue;
            }
            let mut total = Compte::default();
            for compte in entree.comptes.iter() {
                total.fusionne(compte);
            }
            if total.vide() {
                continue;
            }
            sortie[ecrits] = (entree.pid, total);
            ecrits += 1;
        }
        // Tri par insertion : seize entrees au plus, et aucune allocation.
        for i in 1..ecrits {
            let mut j = i;
            while j > 0 && sortie[j - 1].1.total_ns < sortie[j].1.total_ns {
                sortie.swap(j - 1, j);
                j -= 1;
            }
        }
        ecrits
    }
}
