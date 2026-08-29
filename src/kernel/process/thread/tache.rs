/// Un fil d'execution utilisateur.
pub struct Task {
    pub tid: u32,
    pub process: Arc<Process>,
    pub state: TaskState,
    /// Classe d'ordonnancement. Voir [`Priorite`].
    pub priorite: Priorite,
    // BOUCHAUD_SMP_NG2_THREAD_BALANCER_TLB_V1
    /// Masque d'affinite par THREAD. Les taches user naissent sur tous les CPU
    /// online; les fils noyau restent CPU0.
    pub affinity_mask: u64,
    /// Proprietaire logique de la runqueue quand la tache est Ready.
    pub runq_cpu: u8,
    /// Dernier CPU sur lequel la tache a reellement execute.
    pub last_cpu: u8,
    /// CPU qui possede encore l'execution/la pile de cette tache.
    ///
    /// Pendant une commutation sortante, la valeur RESTE >= 0 jusqu'a la
    /// confirmation post-switch. Ainsi aucun autre CPU ne peut republier la
    /// tache tant que l'ancien CPU utilise encore physiquement sa pile.
    pub on_cpu: i8,
    /// Vrai entre prepare_switch_handoff() et complete_switch_handoff().
    /// Protege par le BKL ; ce n'est pas une primitive atomique autonome.
    switching_out: bool,
    /// Derniere migration effective, pour imposer une residence cache minimale.
    pub last_migration_ns: u64,
    /// Runtime recent lisse, utilise comme estimation du poids de la tache.
    pub recent_runtime_ns: u64,
    /// Debut de la tranche courante.
    pub slice_start_ns: u64,
    pub last_account_ns: u64,
    pub user_cpu_ns: u64,
    pub kernel_cpu_ns: u64,
    pub cpu_ns: [u64; MAX_CPUS],
    pub in_kernel: bool,
    pub context_switches: u64,
    pub migrations: u64,
    /// Etat ring 3 quand la tache n'est pas en cours d'execution.
    pub frame: TrapFrame,
    /// Contexte noyau (pile) pour le changement de tache.
    pub ctx: Context,
    /// Pile noyau privee.
    kstack: Vec<u8>,
    pub kstack_top: u64,
    /// Zone `fxsave` (512 octets alignes 16) pour l'etat FPU/SSE.
    fpu: Vec<u8>,
    fpu_area: u64,
    /// Base FS (TLS de la libc) propre au thread.
    pub fs_base: u64,
    /// Adresse ecrite a la mort du thread (`set_tid_address`), pour pthread_join.
    pub clear_child_tid: u64,
    /// Cle du futex attendu, si la tache est bloquee dessus.
    pub futex_key: u64,
    /// Identite d'une WaitQueue noyau, 0 lorsqu'aucune attente n'est armee.
    pub wait_queue_key: usize,
    /// Deadline monotone en nanosecondes (0 = pas de sommeil).
    pub wake_deadline_ns: u64,
    /// La tache attend la fin d'un processus fils (`wait4`).
    pub waiting_for_child: bool,
    /// La tache n'a pas encore rejoint le ring 3.
    pub fresh: bool,
    /// Ticks du timer pendant lesquels cette tache avait la main.
    ///
    /// C'est un profileur par echantillonnage, et le plus simple qui soit : a
    /// chaque IRQ0 — mille fois par seconde — on incremente le compteur de la
    /// tache courante. Sur mille echantillons, la proportion est le temps
    /// processeur, a la precision du tick pres. Cela ne coute qu'une addition
    /// dans un gestionnaire d'interruption qui existe deja, et cela repond a la
    /// seule question qu'on se pose devant une machine lente : qui consomme.
    pub ticks_cpu: u64,
    /// Fil noyau : ne part jamais en ring 3, garde l'espace d'adressage du
    /// noyau, et n'est jamais preempte (l'IRQ0 ne commute que depuis ring 3).
    ///
    /// Le gestionnaire de fenetres en est un. Il pouvait rester sur le fil du
    /// shell tant qu'il lancait ses programmes de facon synchrone ; des lors
    /// qu'il doit **composer pendant** que le navigateur tourne, il lui faut
    /// une place dans l'ordonnanceur comme a tout le monde.
    pub noyau: bool,
    /// Fonction du fil noyau. Elle ne rend jamais la main : elle se termine par
    /// [`exit_current`].
    entree_noyau: Option<fn() -> !>,
}

