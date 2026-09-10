/// Le motif ecrit dans la PAGE DE GARDE de chaque pile noyau.
///
/// # Ce que la garde attrape, et ou est le temoin
///
/// La pile descend. Un debordement touche donc d'abord le HAUT de la garde,
/// juste sous le premier octet utilisable -- et c'est la que sont relus les
/// `CANARI_MOTS` mots a chaque election de la tache. Le canari precedent vivait
/// au BAS de l'allocation, c'est-a-dire a l'endroit qu'un debordement atteint
/// en DERNIER : il ne temoignait que des depassements de plus de soixante
/// kibioctets.
///
/// Le reste de la page porte le meme motif et se relit entierement quand une
/// rupture est constatee : la PROFONDEUR du debordement se lit alors dans le
/// nombre de mots reecrits, ce qui distingue « quelques centaines d'octets de
/// trop » de « `RSP` s'est perdu ».
///
/// Un double fault a ete observe sur la machine de reference avec un `RSP` qui
/// ne designait aucune pile ; sans ce motif, rien ne permet de distinguer « la
/// pile a deborde » de « quelqu'un a ecrase RSP ».
///
/// La valeur est volontairement non nulle et non repetitive : une zone remise a
/// zero, ou remplie d'un octet unique, lui ressemblerait par accident.
pub const CANARI_PILE: u64 = 0xB0C4_0D5A_FEED_1E55;

/// Mots relus a CHAQUE election de la tache, en haut de la page de garde.
///
/// Huit plutot qu'un : un debordement n'ecrit pas forcement le tout premier
/// mot, et une ecriture qui saute par-dessus un seul temoin passerait
/// inapercue. La page entiere n'est pas relue a chaque commutation -- cinq
/// cent douze lectures par changement de contexte se paieraient sur la latence
/// que le chantier ordonnanceur cherche a reduire.
pub const CANARI_MOTS: usize = 8;

/// Un fil d'execution utilisateur.
pub struct Task {
    pub tid: u32,
    pub process: Arc<Process>,
    pub state: EtatAtomique,
    pub priorite: PrioriteAtomique,
    pub affinity_mask: u64,
    pub runq_cpu: CoeurAtomique,
    pub last_cpu: CoeurAtomique,
    pub on_cpu: CoeurSigneAtomique,
    switching_out: DrapeauAtomique,
    pub last_migration_ns: EcheanceAtomique,
    pub recent_runtime_ns: EcheanceAtomique,
    pub slice_start_ns: EcheanceAtomique,
    /// Instant ou la tache est devenue runnable sans encore avoir de CPU.
    /// Zero signifie "pas en file". Sert a la latence wake/ready-to-run NG.
    pub ready_since_ns: EcheanceAtomique,
    pub last_account_ns: EcheanceAtomique,
    pub user_cpu_ns: EcheanceAtomique,
    pub kernel_cpu_ns: EcheanceAtomique,
    pub cpu_ns: [EcheanceAtomique; MAX_CPUS],
    pub in_kernel: DrapeauAtomique,
    pub context_switches: EcheanceAtomique,
    pub migrations: EcheanceAtomique,
    pub frame: TrapFrame,
    pub ctx: Context,
    kstack: Vec<u8>,
    pub kstack_top: u64,
    fpu: Vec<u8>,
    fpu_area: u64,
    pub fs_base: u64,
    pub clear_child_tid: u64,
    pub futex_key: EcheanceAtomique,
    pub wait_queue_key: CleAtomique,
    pub wake_deadline_ns: EcheanceAtomique,
    pub waiting_for_child: DrapeauAtomique,
    pub fresh: bool,
    pub ticks_cpu: EcheanceAtomique,
    pub noyau: bool,
    /// Cette tache noyau peut-elle etre executee par un autre coeur que le zero ?
    ///
    /// Faux par defaut, et ce defaut n'est pas de la prudence de facade : les
    /// taches noyau historiques -- le bureau au premier rang -- supposent le
    /// coeur zero, et leur affinite est ecrite en dur a l'enregistrement.
    ///
    /// Le mettre a vrai est une DECLARATION : ce travail ne depend d'aucun
    /// etat propre au coeur zero. Un travailleur ainsi marque peut etre pris
    /// par n'importe quel coeur en ligne, ce qui est la seule facon qu'il ait
    /// de tourner quand le coeur zero n'est pas, lui, dans l'ordonnanceur.
    pub migrable: bool,
    entree_noyau: Option<fn() -> !>,
}

