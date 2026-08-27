//! Preuve hote de la re-entree par interruption sur la file d'execution.
//!
//! # Le panic reproduit au runtime
//!
//!     src/arch/x86_64/cpu_local.rs:235
//!     SpinLock recursive acquisition on CPU 0
//!     task=21 pid=9 nom=/bo-navigateur in_kernel=true syscall=poll
//!     BKL owner_token=1 depth=6 coherent=true
//!
//! Le gros verrou etait COHERENT : ce n'est pas une corruption de profondeur.
//! C'est un verrou tournant ordinaire repris par un gestionnaire d'interruption
//! sur le CPU qui le detenait deja.
//!
//! # Le chemin exact
//!
//!     IRQ clavier/souris (8042)
//!       -> push_scancode / mouse::handle_byte
//!       -> kernel::sync::signale_interface(...)   [reveil du compositeur]
//!       -> WaitQueue::wake_all
//!       -> task::wake_wait_queue
//!       -> task::publish_ready
//!       -> CpuLocal::enqueue        <-- reprend la file deja detenue
//!
//! Les trois dernieres transitions de l'enregistreur de vol sont litteralement
//! cette chaine : REENTER 3->4 (le garde du gestionnaire), REENTER 4->5
//! (`enter_bkl` dans `wake_all`), REENTER 5->6 (`wake_wait_queue`).
//!
//! # Pourquoi le gros verrou ne protege pas
//!
//! Il appartient a un CPU, pas a une tache, et `smp_lock::enter()` est
//! REENTRANTE. Un gestionnaire d'interruption qui le reprend sur un CPU qui le
//! detient deja obtient un garde valide et continue -- c'est le comportement
//! voulu, et c'est justement ce qui amene le gestionnaire jusqu'a `enqueue`.
//!
//! Lance par `tools/smp/test-runqueue-irq.sh`.

/// Ce que `SpinLock::lock` detecte.
#[derive(Debug, PartialEq, Eq)]
enum Prise {
    Ok,
    /// Le `debug_assert!` de recursion : ce CPU detient deja ce verrou.
    Recursive,
}

/// Un CPU, reduit a ce qui compte ici : son drapeau d'interruption et son
/// interruption en attente.
struct Cpu {
    interruptions_actives: bool,
    irq_en_attente: bool,
}

impl Cpu {
    fn neuf() -> Self {
        Self { interruptions_actives: true, irq_en_attente: false }
    }
}

/// Le verrou, avec ou sans masquage des interruptions.
struct Verrou {
    proprietaire: Option<usize>,
    /// `true` pour `SpinLockIrq`, `false` pour `SpinLock`.
    masque_les_interruptions: bool,
    /// Etat `IF` a restaurer au relachement.
    restaurer: bool,
}

impl Verrou {
    fn ordinaire() -> Self {
        Self { proprietaire: None, masque_les_interruptions: false, restaurer: false }
    }

    fn masquant() -> Self {
        Self { proprietaire: None, masque_les_interruptions: true, restaurer: false }
    }

    fn prend(&mut self, cpu_id: usize, cpu: &mut Cpu) -> Prise {
        if self.proprietaire == Some(cpu_id) {
            return Prise::Recursive;
        }
        if self.masque_les_interruptions {
            self.restaurer = cpu.interruptions_actives;
            cpu.interruptions_actives = false;
        }
        self.proprietaire = Some(cpu_id);
        Prise::Ok
    }

    fn rend(&mut self, cpu: &mut Cpu) {
        self.proprietaire = None;
        if self.masque_les_interruptions && self.restaurer {
            cpu.interruptions_actives = true;
        }
    }
}

/// Le materiel leve une interruption. Elle n'est prise que si `IF` est actif ;
/// sinon elle reste en attente dans l'APIC local -- elle n'est jamais perdue.
fn leve_interruption(cpu: &mut Cpu) -> bool {
    if cpu.interruptions_actives {
        true
    } else {
        cpu.irq_en_attente = true;
        false
    }
}

/// Le gestionnaire d'interruption, reduit a ce qu'il fait de fautif : reprendre
/// la file d'execution pour y publier une tache reveillee.
fn gestionnaire_irq(verrou: &mut Verrou, cpu_id: usize, cpu: &mut Cpu) -> Prise {
    let prise = verrou.prend(cpu_id, cpu);
    if prise == Prise::Ok {
        verrou.rend(cpu);
    }
    prise
}

// ===========================================================================

/// LE TEST QUI REPRODUIT LE PANIC.
///
/// Un `SpinLock` ordinaire ne masque rien : l'interruption tombe pendant la
/// section critique, le gestionnaire reprend la meme file, et la recursion est
/// detectee.
#[test]
fn sans_masquage_l_irq_reprend_la_file_recursivement() {
    let mut cpu = Cpu::neuf();
    let mut file = Verrou::ordinaire();

    // Contexte tache : le poll de /bo-navigateur entre dans un accesseur.
    assert_eq!(file.prend(0, &mut cpu), Prise::Ok);

    // L'IRQ 8042 tombe ICI, pendant `contains()` / `push()` / `remove(0)`.
    assert!(
        leve_interruption(&mut cpu),
        "sans masquage, l'interruption est prise immediatement"
    );
    assert_eq!(
        gestionnaire_irq(&mut file, 0, &mut cpu),
        Prise::Recursive,
        "SpinLock recursive acquisition on CPU 0 -- le panic observe"
    );
}

/// LE CONTRAT CORRIGE : `SpinLockIrq` masque pour la duree de la section
/// critique. L'interruption ne peut pas tomber dedans.
#[test]
fn avec_masquage_l_irq_ne_peut_pas_reprendre_la_file() {
    let mut cpu = Cpu::neuf();
    let mut file = Verrou::masquant();

    assert_eq!(file.prend(0, &mut cpu), Prise::Ok);
    assert!(!cpu.interruptions_actives, "la section critique masque IF");

    assert!(
        !leve_interruption(&mut cpu),
        "l'interruption ne peut pas etre prise maintenant"
    );
    assert!(cpu.irq_en_attente, "elle reste en attente, elle n'est pas perdue");

    file.rend(&mut cpu);
    assert!(cpu.interruptions_actives, "IF est restaure exactement");
}

/// AUCUN REVEIL PERDU. C'est la moitie du contrat qu'un `try_lock` ou un
/// `return` silencieux auraient cassee : l'interruption differee est bel et
/// bien delivree, et la tache reveillee entre bien en file.
#[test]
fn l_interruption_differee_est_delivree_et_publie_sa_tache() {
    let mut cpu = Cpu::neuf();
    let mut file = Verrou::masquant();

    file.prend(0, &mut cpu);
    leve_interruption(&mut cpu); // mise en attente
    file.rend(&mut cpu);

    // Le materiel delivre ce qui etait en attente des que IF revient.
    assert!(cpu.irq_en_attente);
    let delivree = cpu.irq_en_attente && cpu.interruptions_actives;
    assert!(delivree, "l'IRQ en attente est delivree au relachement");
    cpu.irq_en_attente = false;

    assert_eq!(
        gestionnaire_irq(&mut file, 0, &mut cpu),
        Prise::Ok,
        "le gestionnaire publie sa tache normalement : rien n'est perdu"
    );
}

/// Le masquage ne doit pas empecher un AUTRE CPU de prendre la file : ce n'est
/// pas un verrou global, et le serialiser par CPU casserait le vol de taches.
#[test]
fn le_masquage_n_empeche_pas_un_autre_cpu() {
    let mut cpu0 = Cpu::neuf();
    let mut cpu1 = Cpu::neuf();
    let mut file = Verrou::masquant();

    file.prend(0, &mut cpu0);
    file.rend(&mut cpu0);
    assert_eq!(
        file.prend(1, &mut cpu1),
        Prise::Ok,
        "un autre CPU prend la file sans difficulte une fois libre"
    );
}

/// Deux prises SEQUENTIELLES par le meme CPU sont legitimes : `dequeue` puis
/// `enqueue` dans `pick_next`, par exemple. Seule la re-entree pendant la
/// section critique est fautive -- c'est bien un probleme d'interruption, pas
/// un probleme d'ordre des appels.
#[test]
fn deux_prises_sequentielles_du_meme_cpu_sont_legitimes() {
    let mut cpu = Cpu::neuf();
    let mut file = Verrou::ordinaire();

    assert_eq!(file.prend(0, &mut cpu), Prise::Ok);
    file.rend(&mut cpu);
    assert_eq!(
        file.prend(0, &mut cpu),
        Prise::Ok,
        "le proprietaire est efface au relachement : aucun faux positif"
    );
}

/// Sans masquage, l'imbrication peut aller plus loin qu'un niveau : c'est ce
/// que montrait l'enregistreur de vol, avec trois REENTER consecutifs du gros
/// verrou avant le panic. Le premier niveau suffit deja a paniquer.
#[test]
fn la_re_entree_est_detectee_des_le_premier_niveau() {
    let mut cpu = Cpu::neuf();
    let mut file = Verrou::ordinaire();

    file.prend(0, &mut cpu);
    // Le gestionnaire prend le gros verrou de facon REENTRANTE -- c'est permis,
    // il appartient au CPU. Puis il arrive a la file, qui ne l'est pas.
    let profondeur_bkl_avant = 3;
    let profondeur_bkl_apres = profondeur_bkl_avant + 3; // les trois REENTER
    assert_eq!(profondeur_bkl_apres, 6, "la profondeur observee au FAULT");

    assert_eq!(
        gestionnaire_irq(&mut file, 0, &mut cpu),
        Prise::Recursive,
        "un gros verrou coherent n'empeche pas la re-entree d'un verrou tournant"
    );
}

// ===========================================================================
// Garde sur le code reel
// ===========================================================================

/// Les tests ci-dessus expliquent POURQUOI la file doit masquer les
/// interruptions ; ils ne peuvent pas verifier qu'elle le fait, parce que
/// `cpu_local.rs` ne se compile pas sur l'hote -- il touche l'APIC, le GDT et
/// le gros verrou.
///
/// Celui-ci lit la declaration reelle. Revenir a un `SpinLock` ordinaire
/// reintroduirait exactement le panic observe, et ce test echouerait.
#[test]
fn la_file_d_execution_reste_un_verrou_masquant() {
    const SOURCE: &str = include_str!("../../src/arch/x86_64/cpu_local.rs");

    assert!(
        SOURCE.contains("run_queue: SpinLockIrq<Vec<usize>>"),
        "la file d'execution doit rester un SpinLockIrq : elle est atteignable \
         depuis l'IRQ 8042 via le reveil du compositeur"
    );
    assert!(
        !SOURCE.contains("run_queue: SpinLock<"),
        "un SpinLock ordinaire sur la file rouvre la re-entree par interruption"
    );
    assert!(
        SOURCE.contains("run_queue: SpinLockIrq::new(Vec::new())"),
        "et sa construction doit suivre la declaration"
    );
}

/// L'assertion de recursion de `SpinLock` doit rester en place : c'est elle qui
/// a rendu ce bug visible au lieu de le laisser corrompre la file en silence.
#[test]
fn l_assertion_de_recursion_du_verrou_tournant_reste_en_place() {
    const SOURCE: &str = include_str!("../../src/kernel/sync/spinlock.rs");

    assert!(
        SOURCE.contains("SpinLock recursive acquisition on CPU"),
        "l'assertion de recursion ne doit jamais etre retiree pour faire taire \
         un panic : elle est le seul temoin de ce genre de faute"
    );
    assert!(
        SOURCE.contains("self.lock.owner_cpu.store(NO_OWNER"),
        "le proprietaire doit etre efface au relachement, sinon l'assertion \
         produirait des faux positifs sur deux prises sequentielles"
    );
}
