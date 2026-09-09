/// Le motif ecrit au PIED de chaque pile noyau.
///
/// # Pourquoi un canari plutot qu'une page de garde
///
/// Une page de garde est meilleure : elle transforme le debordement en faute
/// AU MOMENT ou il se produit, sur l'instruction fautive. Elle exige en
/// revanche que les piles viennent du gestionnaire de memoire virtuelle, avec
/// une page non mappee en dessous -- ce que ce noyau ne fait pas encore : ses
/// piles sont des allocations de 64 Kio du tas.
///
/// Le canari ne coute rien et repond a la question qui manquait. Un double
/// fault a ete observe sur la machine de reference avec un `RSP` qui ne
/// designait aucune pile ; sans ce motif, rien ne permet de distinguer « la
/// pile a deborde » de « quelqu'un a ecrase RSP ». Le canari le dit.
///
/// La valeur est volontairement non nulle et non repetitive : une zone remise a
/// zero, ou remplie d'un octet unique, ressemblerait au canari par accident.
pub const CANARI_PILE: u64 = 0xB0C4_0D5A_FEED_1E55;

/// Octets du pied de pile couverts par le canari.
///
/// Huit mots plutot qu'un : un debordement n'ecrit pas forcement le tout
/// premier mot, et une ecriture qui saute par-dessus un seul temoin passerait
/// inapercue.
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
    entree_noyau: Option<fn() -> !>,
}

