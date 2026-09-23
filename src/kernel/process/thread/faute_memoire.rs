/// Ressources partagees par tous les threads d'un meme programme.
/// Peuple a la demande la page fautive, si elle a ete promise.
///
/// Rend `true` si la faute est reparee et que l'instruction peut etre reprise.
///
/// C'est la contrepartie de `MAP_NORESERVE` (voir `abi::mem::sys_mmap`) : la
/// plage a ete promise sans etre peuplee, et c'est ici qu'elle le devient, une
/// page a la fois, avec les droits enregistres a la reservation.
///
/// La fonction est volontairement stricte : elle ne peuple **que** ce qui a ete
/// promis. Une faute hors de toute promesse reste une faute, et le processus
/// meurt comme avant — sans quoi le noyau transformerait chaque dereference de
/// pointeur nul en allocation silencieuse, et l'on perdrait le seul mecanisme
/// qui signale ces defauts.
static FAULTS_ZERO: AtomicU64 = AtomicU64::new(0);
static FAULTS_FILE: AtomicU64 = AtomicU64::new(0);
static FAULT_WAITS: AtomicU64 = AtomicU64::new(0);
static FAULT_RESOLVED: AtomicU64 = AtomicU64::new(0);
static FAULT_RETRY: AtomicU64 = AtomicU64::new(0);
static FAULT_INVALID: AtomicU64 = AtomicU64::new(0);
static FAULT_IO_ERROR: AtomicU64 = AtomicU64::new(0);
static FAULT_RETIRED: AtomicU64 = AtomicU64::new(0);
static PF_BKL_ENTERS: AtomicU64 = AtomicU64::new(0);
static FAULT_REGISTRY_PEAK: AtomicU64 = AtomicU64::new(0);
static FAULT_RETRY_YIELDS: AtomicU64 = AtomicU64::new(0);
static FAULT_RETRY_MAX_CHAIN: AtomicU64 = AtomicU64::new(0);

// BOUCHAUD_C24_FAUTES_PAR_PROCESSUS
//
// Le livre de comptes par processus et par categorie. L'arithmetique vit dans
// `crate::kernel::fautes`, qui ne depend de rien et se verifie sur l'hote
// (`tools/process/test_fautes.rs`) ; ce qui suit n'est que le raccordement.
//
// `try_lock` ET JAMAIS `lock`, et ce n'est pas une precaution de style.
//
// Ce chemin-ci est celui de CHAQUE faute de page de la machine. Y poser un
// verrou bloquant serialiserait seize processeurs sur un compteur de
// diagnostic : la mesure creerait la lenteur qu'elle pretend observer, et
// personne ne le verrait puisque l'outil de mesure serait le coupable. Un
// echantillon perdu sous contention ne coute rien -- a condition de le DIRE,
// et c'est le role de `FAUTES_NON_COMPTEES`.
static LIVRE_FAUTES: SpinLock<crate::kernel::fautes::Journal> =
    SpinLock::new(crate::kernel::fautes::Journal::neuf());

/// Les fautes qu'on a renonce a compter faute d'avoir eu le verrou.
///
/// Sans ce compteur, un livre qui perd la moitie de ses echantillons rend des
/// chiffres deux fois trop petits sans qu'aucune ligne ne le signale -- et on
/// conclurait que les fautes ne coutent rien.
static FAUTES_NON_COMPTEES: AtomicU64 = AtomicU64::new(0);

/// Les notes REFUSEES parce que leur faute etait deja notee.
///
/// BOUCHAUD_C26_UNE_FAUTE_UN_SEUL_COMPTE
///
/// Ce compteur n'est pas une statistique : c'est une ALARME. Sa valeur
/// normale est zero, et toute valeur non nulle dit qu'un chemin de faute
/// note deux fois la meme duree -- exactement le defaut qu'avait `Attente`.
///
/// Il existe parce que la somme des categories ne peut PAS trahir ce defaut :
/// le total d'un processus EST la somme de ses categories, par construction
/// du livre. Une verification qui les compare ne verifie donc rien -- elle a
/// ete ecrite, et elle a laisse passer la regression reintroduite expres.
/// Il faut un temoin porte par la faute elle-meme, et c'est `Note`.
static FAUTES_DOUBLES: AtomicU64 = AtomicU64::new(0);

/// Les fautes PRESENTEES au livre, comptees ou non.
///
/// BOUCHAUD_C26_TAUX_DE_PERTE
///
/// `FAUTES_NON_COMPTEES` seul ne se lit pas : « douze mille perdues » ne dit
/// rien sans savoir si c'est douze mille sur treize mille ou sur douze
/// millions. Le denominateur est ce qui transforme ce compteur en decision --
/// sous un pour cent, le `try_lock` est le bon compromis ; au-dela, il faut
/// un livre par processeur.
static FAUTES_PRESENTEES: AtomicU64 = AtomicU64::new(0);

/// Enregistre une faute dont la categorie est connue.
///
/// `debut_ns` est l'instant ou la faute est entree dans la resolution. La
/// duree mesuree couvre donc l'attente des verrous et le travail de
/// chargement : c'est ce que le processus a reellement perdu, et non ce que
/// le noyau a passe a travailler.
fn note_faute(pid: u32, categorie: crate::kernel::fautes::Categorie, debut_ns: u64) {
    let maintenant = crate::kernel::timer::monotonic_ns();
    let duree = maintenant.saturating_sub(debut_ns);
    FAUTES_PRESENTEES.fetch_add(1, Ordering::Relaxed);
    match LIVRE_FAUTES.try_lock() {
        Some(mut livre) => livre.note(pid, categorie, duree, maintenant),
        None => {
            FAUTES_NON_COMPTEES.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Le total d'un processus, toutes categories confondues.
pub fn fautes_du_processus(pid: u32) -> Option<crate::kernel::fautes::Compte> {
    let livre = LIVRE_FAUTES.try_lock()?;
    let total = livre.total(pid);
    if total.vide() {
        return None;
    }
    Some(total)
}

/// Le detail par categorie, pour un processus.
///
/// BOUCHAUD_C34_LES_FAUTES_PAR_PHASE
///
/// `fautes_du_processus` rend le TOTAL et `faute_dominante` la categorie la
/// plus lourde. Ni l'un ni l'autre ne permet de repondre a la question posee :
/// « les vingt-trois secondes avant `main` sont-elles du chargement d'image
/// (fautes FICHIER) ou de l'allocation (fautes ZERO) ? » Les deux se soignent
/// a des endroits opposes -- lecture anticipee d'un cote, dimensionnement des
/// arenes de l'autre.
///
/// Rend `None` quand le livre ne sait rien de ce processus, et non un tableau
/// de zeros : le livre est borne et il chasse ; un zero affirmerait « ce
/// processus ne faute pas » la ou il faudrait dire « je ne sais pas ».
pub fn fautes_par_categorie(
    pid: u32,
) -> Option<[crate::kernel::fautes::Compte; crate::kernel::fautes::CATEGORIES]> {
    let livre = LIVRE_FAUTES.try_lock()?;
    if livre.total(pid).vide() {
        return None;
    }
    let mut sortie = [crate::kernel::fautes::Compte::default(); crate::kernel::fautes::CATEGORIES];
    for rang in 0..crate::kernel::fautes::CATEGORIES {
        if let Some(categorie) = crate::kernel::fautes::Categorie::depuis_rang(rang) {
            sortie[rang] = livre.compte(pid, categorie);
        }
    }
    Some(sortie)
}

/// Le droit de noter UNE faute, et une seule fois.
///
/// BOUCHAUD_C26_UNE_FAUTE_UN_SEUL_COMPTE
///
/// Le chemin de faute avait l'instant de depart sous la main -- un simple
/// `u64` -- et pouvait donc le presenter au livre autant de fois qu'il
/// passait devant un appel. Il l'a fait : `Attente` etait notee a CHAQUE
/// tour de la boucle d'attente, puis une seconde fois par le chargeur si le
/// dormeur devenait chargeur. La meme duree, partant du meme instant,
/// comptee deux fois et parfois plus.
///
/// Un `u64` ne peut pas refuser d'etre relu. Ce jeton le peut : il porte
/// l'instant de depart ET le fait que la note a deja ete posee. Le second
/// appel ne corrompt plus le livre -- il leve `FAUTES_DOUBLES`.
///
/// La regle qu'il rend indiscutable : une faute, une note. `Attente` ne
/// decrit que les fautes dont le cout ENTIER a ete l'attente. Une faute qui
/// finit par charger appartient a sa categorie, et son attente est deja
/// comprise dans sa duree.
pub struct Note {
    pid: u32,
    debut_ns: u64,
    posee: bool,
}

impl Note {
    /// Ouvre le compte d'une faute : l'horloge part ici.
    pub fn ouvre(pid: u32) -> Note {
        Note { pid, debut_ns: crate::kernel::timer::monotonic_ns(), posee: false }
    }

    /// Classe cette faute. Le deuxieme appel est refuse et signale.
    pub fn pose(&mut self, categorie: crate::kernel::fautes::Categorie) {
        if self.posee {
            FAUTES_DOUBLES.fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.posee = true;
        note_faute(self.pid, categorie, self.debut_ns);
    }
}

/// Combien de notes ont ete refusees parce que leur faute etait deja notee.
///
/// Sa valeur normale est zero. Toute autre valeur est une regression.
pub fn notes_doubles() -> u64 {
    FAUTES_DOUBLES.load(Ordering::Relaxed)
}

/// La categorie qui coute le plus de temps a ce processus, et son compte.
pub fn faute_dominante(
    pid: u32,
) -> Option<(crate::kernel::fautes::Categorie, crate::kernel::fautes::Compte)> {
    LIVRE_FAUTES.try_lock()?.categorie_dominante(pid)
}

/// Combien de fautes n'ont pas pu etre comptees, et combien de processus ont
/// ete chasses du livre. Les deux disent la meme chose : que les chiffres
/// rendus sont un plancher, pas un total.
pub fn fautes_incompletes() -> (u64, u64, u64) {
    let presentees = FAUTES_PRESENTEES.load(Ordering::Relaxed);
    let non_comptees = FAUTES_NON_COMPTEES.load(Ordering::Relaxed);
    let chasses = LIVRE_FAUTES.try_lock().map(|livre| livre.chasses).unwrap_or(0);
    (presentees, non_comptees, chasses)
}

/// La part des fautes perdues, en pour mille des fautes presentees.
///
/// EN POUR MILLE, comme la part de temps passe en faute : une perte de trois
/// pour mille est un bruit acceptable, une perte de trois pour cent ne l'est
/// pas, et l'arrondi entier du pourcent confondrait les deux.
pub fn taux_de_perte_pour_mille() -> u64 {
    let presentees = FAUTES_PRESENTEES.load(Ordering::Relaxed);
    if presentees == 0 {
        return 0;
    }
    FAUTES_NON_COMPTEES.load(Ordering::Relaxed).saturating_mul(1000) / presentees
}

/// Ecrit le livre des fautes sur la sortie courante.
///
/// C'est la commande `fautes` du shell. Elle existe parce que le livre ne
/// servait a rien tant qu'il n'etait lisible que depuis la fenetre Services :
/// celle-ci ne publie que les processus `browser.*`, alors que le livre les
/// compte TOUS -- et sur une machine ou le navigateur est lent, le processus
/// qui paie n'est pas forcement celui qu'on soupconne.
///
/// Elle ne prend le verrou qu'une fois, pour une copie bornee, et rend la
/// main : un diagnostic ne doit pas retenir le chemin de faute de la machine.
pub fn ecris_les_fautes() {
    use crate::kernel::fautes::{Categorie, Compte, CATEGORIES, PROCESSUS_MAX};

    let mut classement = [(0u32, Compte::default()); PROCESSUS_MAX];
    let mut details = [[Compte::default(); CATEGORIES]; PROCESSUS_MAX];
    let (combien, chasses) = {
        let Some(livre) = LIVRE_FAUTES.try_lock() else {
            crate::println!("fautes : le livre est occupe, reessayer");
            return;
        };
        let combien = livre.classement(&mut classement);
        for rang in 0..combien {
            for index in 0..CATEGORIES {
                if let Some(categorie) = Categorie::depuis_rang(index) {
                    details[rang][index] = livre.compte(classement[rang].0, categorie);
                }
            }
        }
        (combien, livre.chasses)
    };

    let (presentees, non_comptees, _) = fautes_incompletes();
    if combien == 0 {
        crate::println!("fautes : aucune faute enregistree pour l'instant");
    } else {
        crate::println!("  PID   FAUTES     TOTAL      PIRE   DOMINANTE");
        for rang in 0..combien {
            let (pid, total) = classement[rang];
            // La dominante est celle qui coute du TEMPS, et non celle qui
            // compte le plus de fautes : c'est le temps que l'utilisateur
            // ressent.
            let mut pire_categorie = "—";
            let mut pire_total = 0u64;
            for index in 0..CATEGORIES {
                let compte = details[rang][index];
                if compte.vide() || compte.total_ns <= pire_total {
                    continue;
                }
                pire_total = compte.total_ns;
                if let Some(categorie) = Categorie::depuis_rang(index) {
                    pire_categorie = categorie.nom();
                }
            }
            crate::println!(
                "  {:>3}  {:>7}  {:>6} ms  {:>5} us   {} ({} ms)",
                pid,
                total.nombre,
                total.total_ns / 1_000_000,
                total.pire_ns / 1_000,
                pire_categorie,
                pire_total / 1_000_000,
            );
            // LE DETAIL PAR CATEGORIE, et il sert a une verification precise.
            //
            // La somme des categories doit faire le total. Si elle le
            // depasse, une meme faute est comptee deux fois -- c'est
            // exactement le defaut qu'avait `Attente`, notee a chaque tour de
            // la boucle d'attente PUIS une seconde fois par le chargeur.
            for index in 0..CATEGORIES {
                let compte = details[rang][index];
                if compte.vide() {
                    continue;
                }
                let Some(categorie) = Categorie::depuis_rang(index) else { continue };
                crate::println!(
                    "         {:<9} n={:<6} total={:>5} ms  pire={:>5} us",
                    categorie.nom(),
                    compte.nombre,
                    compte.total_ns / 1_000_000,
                    compte.pire_ns / 1_000,
                );
            }
        }
    }

    // LES CHIFFRES SONT UN PLANCHER, ET IL FAUT LE DIRE.
    //
    // Le livre est borne et il prend son verrou sans attendre : il chasse les
    // processus inactifs et laisse tomber les echantillons pris pendant qu'un
    // autre coeur ecrit. Taire ces deux nombres donnerait un total qu'on
    // croirait complet.
    crate::println!(
        "  suivis={} chasses={} presentees={} non_comptees={} perte_pour_mille={} doubles={}",
        combien,
        chasses,
        presentees,
        non_comptees,
        taux_de_perte_pour_mille(),
        notes_doubles(),
    );
}

/// Un processus est mort : ses comptes partent avec lui.
///
/// Un PID se reutilise. Laisser les comptes de l'ancien occupant ferait
/// apparaitre un WebContent neuf avec les fautes de celui qui vient de
/// planter -- et l'on chercherait le defaut dans le mauvais processus.
pub fn oublie_les_fautes(pid: u32) {
    if let Some(mut livre) = LIVRE_FAUTES.try_lock() {
        livre.oublie(pid);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultOutcome {
    Resolved,
    Retry,
    Invalid,
    IoError,
    Retired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MappingPart {
    id: u64,
    start: u64,
    end: u64,
    drapeaux: u64,
    backing: PromesseBacking,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MappingToken {
    effective_id: u64,
    parts: Vec<MappingPart>,
}

fn mapping_token(regions: &[Promesse], page: u64) -> Option<MappingToken> {
    let page_end = page.checked_add(crate::kernel::vmm::PAGE_SIZE)?;
    let effective_id = crate::kernel::vma::trouve(regions, page)?.id;
    let parts = regions
        .iter()
        .filter(|region| region.chevauche(page, page_end))
        .map(|region| MappingPart {
            id: region.id,
            start: region.debut.max(page),
            end: region.fin.min(page_end),
            drapeaux: region.drapeaux,
            backing: region.backing,
        })
        .collect();
    Some(MappingToken { effective_id, parts })
}

include!("faute_cluster.rs");

#[derive(Clone, PartialEq, Eq)]
enum FaultPageState {
    Missing,
    Loading,
    Present,
    Failed(FaultOutcome),
    Cancelled,
    Retired,
}

struct FaultPage {
    pml4: u64,
    page: u64,
    token: MappingToken,
    state: SpinLock<FaultPageState>,
    waiters: crate::kernel::sync::WaitQueue,
}

static FAULT_PAGES: SpinLock<Vec<Arc<FaultPage>>> = SpinLock::new(Vec::new());

fn fault_page(pml4: u64, page: u64, token: &MappingToken) -> Arc<FaultPage> {
    let mut pages = FAULT_PAGES.lock();
    if let Some(entry) = pages.iter().find(|entry| {
        entry.pml4 == pml4 && entry.page == page && entry.token == *token
    }) {
        return Arc::clone(entry);
    }
    let entry = Arc::new(FaultPage {
        pml4,
        page,
        token: token.clone(),
        state: SpinLock::new(FaultPageState::Missing),
        waiters: crate::kernel::sync::WaitQueue::new(),
    });
    pages.push(Arc::clone(&entry));
    FAULT_REGISTRY_PEAK.fetch_max(pages.len() as u64, Ordering::Relaxed);
    entry
}

fn forget_fault_page(entry: &Arc<FaultPage>) {
    FAULT_PAGES.lock().retain(|candidate| !Arc::ptr_eq(candidate, entry));
}

pub fn forget_fault_space(pml4: u64) {
    retire_fault_records(|entry| entry.pml4 == pml4);
}

/// Retire only loaders whose virtual page intersects a changed mapping range.
/// Unrelated mmap/brk/mprotect operations therefore cannot cancel this fault.
pub fn retire_fault_range(pml4: u64, start: u64, len: u64) {
    let end = start.saturating_add(len);
    cancel_fault_records(|entry| {
        entry.pml4 == pml4 && entry.page < end
            && entry.page.saturating_add(crate::kernel::vmm::PAGE_SIZE) > start
    });
}

fn transition_fault_records(
    mut predicate: impl FnMut(&FaultPage) -> bool,
    terminal: FaultPageState,
) {
    // Publish the terminal state while every matching entry is still
    // discoverable. A concurrent lookup can therefore never recreate a second
    // loader in the remove-before-cancel window.
    let entries = {
        let registry = FAULT_PAGES.lock();
        let entries: Vec<_> = registry.iter().filter(|entry| predicate(entry)).cloned().collect();
        for entry in &entries {
            *entry.state.lock() = terminal.clone();
        }
        entries
    };
    for entry in &entries {
        entry.waiters.wake_all();
    }
    FAULT_PAGES.lock().retain(|candidate| {
        !entries.iter().any(|entry| Arc::ptr_eq(entry, candidate))
    });
}

fn cancel_fault_records(predicate: impl FnMut(&FaultPage) -> bool) {
    transition_fault_records(predicate, FaultPageState::Cancelled);
}

fn retire_fault_records(predicate: impl FnMut(&FaultPage) -> bool) {
    transition_fault_records(predicate, FaultPageState::Retired);
}

fn record_fault_outcome(outcome: FaultOutcome) {
    match outcome {
        FaultOutcome::Resolved => &FAULT_RESOLVED,
        FaultOutcome::Retry => &FAULT_RETRY,
        FaultOutcome::Invalid => &FAULT_INVALID,
        FaultOutcome::IoError => &FAULT_IO_ERROR,
        FaultOutcome::Retired => &FAULT_RETIRED,
    }
    .fetch_add(1, Ordering::Relaxed);
}

/// Resolve one demand-fault attempt. Mapping replacement is a typed retry,
/// never an accidental SIGSEGV; a genuine missing/protected VMA is Invalid.
pub fn peuple_a_la_demande(adresse: u64, protection_fault: bool) -> FaultOutcome {
    if !crate::kernel::vmm::is_user_addr(adresse) || !in_user_task() || protection_fault {
        record_fault_outcome(FaultOutcome::Invalid);
        return FaultOutcome::Invalid;
    }

    let Some(processus) = current_process_local() else {
        record_fault_outcome(FaultOutcome::Retired);
        return FaultOutcome::Retired;
    };

    // BOUCHAUD_C24_FAUTES_PAR_PROCESSUS
    //
    // L'instant d'ENTREE, et non celui ou le chargement commence. Ce que le
    // processus perd inclut l'attente des verrous et celle d'un autre
    // processeur qui charge la meme page ; ne mesurer que le chargement
    // rendrait un chiffre flatteur qui n'explique aucune saccade.
    let pid_fautif = processus.pid;
    let mut note = Note::ouvre(pid_fautif);

    let page = adresse & !(crate::kernel::vmm::PAGE_SIZE - 1);
    let (fault_pml4, token) = {
        let mm = processus.mm.lock();
        let Some(token) = mapping_token(&mm.promesses, page) else {
            record_fault_outcome(FaultOutcome::Invalid);
            return FaultOutcome::Invalid;
        };
        (mm.space.pml4(), token)
    };
    let record = fault_page(fault_pml4, page, &token);
    // Voir `Note` : le jeton refuse la deuxieme note. Ce drapeau-ci dit
    // seulement s'il y a EU attente, car seule une faute resolue PENDANT
    // l'attente appartient a la categorie `Attente`.
    let mut a_attendu = false;
    loop {
        let mut state = record.state.lock();
        match &*state {
            FaultPageState::Loading => {
                let ticket = record.waiters.ticket();
                drop(state);
                FAULT_WAITS.fetch_add(1, Ordering::Relaxed);
                record.waiters.wait(ticket);
                a_attendu = true;
            }
            FaultPageState::Present => {
                let resolved = processus.mm.lock().space.translate(page).is_some();
                if resolved {
                    if a_attendu {
                        // Cette faute-ci n'a rien charge : elle a attendu, et
                        // un autre processeur l'a resolue. La compter comme un
                        // chargement attribuerait a ce processus le travail
                        // d'un autre, et masquerait la contention -- qui est la
                        // vraie mesure interessante quand six processus
                        // Ladybird partagent des pages.
                        note.pose(crate::kernel::fautes::Categorie::Attente);
                    }
                    record_fault_outcome(FaultOutcome::Resolved);
                    return FaultOutcome::Resolved;
                }
                *state = FaultPageState::Loading;
                break;
            }
            FaultPageState::Missing => {
                *state = FaultPageState::Loading;
                break;
            }
            FaultPageState::Failed(outcome) => {
                let outcome = *outcome;
                drop(state);
                record_fault_outcome(outcome);
                return outcome;
            }
            FaultPageState::Cancelled => {
                drop(state);
                record_fault_outcome(FaultOutcome::Retry);
                return FaultOutcome::Retry;
            }
            FaultPageState::Retired => {
                drop(state);
                record_fault_outcome(FaultOutcome::Retired);
                return FaultOutcome::Retired;
            }
        }
    }

    let loaded = peuple_page_loader(&processus, adresse, fault_pml4, &token, &mut note);
    let outcome = {
        let mut state = record.state.lock();
        match *state {
            FaultPageState::Retired => FaultOutcome::Retired,
            FaultPageState::Cancelled => FaultOutcome::Retry,
            _ => {
                *state = if loaded == FaultOutcome::Resolved {
                    FaultPageState::Present
                } else {
                    FaultPageState::Failed(loaded)
                };
                loaded
            }
        }
    };
    record.waiters.wake_all();
    // The PTE (for Present) or the Arc already held by each waiter is the
    // durable state. Keeping terminal records here leaked one Vec entry per
    // fault and made future lookup O(total faults). Publish, wake, then detach.
    forget_fault_page(&record);
    record_fault_outcome(outcome);
    // Une faute qui ECHOUE a coute du temps elle aussi, et elle en coute
    // souvent plus qu'une qui reussit : c'est le chemin qui finit par tuer le
    // processus. `Retry` et `Retired` n'en sont pas -- la premiere sera
    // rejouee et comptee alors, la seconde appartient a un processus qui
    // n'existe plus.
    if matches!(outcome, FaultOutcome::Invalid | FaultOutcome::IoError) {
        note.pose(crate::kernel::fautes::Categorie::Echec);
    }
    outcome
}

fn peuple_page_loader(
    processus: &Arc<Process>,
    adresse: u64,
    fault_pml4: u64,
    token: &MappingToken,
    note: &mut Note,
) -> FaultOutcome {
    stall_pf_phase(210, adresse);
    stall_pf_phase(211, adresse);
    let mut p = processus.mm.lock();
    let page = adresse & !(crate::kernel::vmm::PAGE_SIZE - 1);
    stall_pf_phase(212, page);
    if p.space.pml4() != fault_pml4 {
        return FaultOutcome::Retired;
    }
    if mapping_token(&p.promesses, page).as_ref() != Some(token) {
        return FaultOutcome::Retry;
    }

    let effective = match crate::kernel::vma::trouve(&p.promesses, page) {
        Some(region) => region,
        None => return FaultOutcome::Invalid,
    };
    if effective.drapeaux & crate::kernel::vmm::PTE_USER == 0 {
        return FaultOutcome::Invalid;
    }
    if p.space.translate(page).is_some() {
        return FaultOutcome::Resolved;
    }

    match effective.backing {
        PromesseBacking::Zero => {
            stall_pf_phase(220, page);
            let zero_id = effective.id;
            let zero_flags = effective.drapeaux;
            if !p.space.map_alloc_accounted(
                page,
                crate::kernel::vmm::PAGE_SIZE,
                zero_flags,
                crate::kernel::vmm::ResidentKind::Anonymous,
            ) {
                return FaultOutcome::IoError;
            }
            FAULTS_ZERO.fetch_add(1, Ordering::Relaxed);
            note.pose(crate::kernel::fautes::Categorie::Zero);
            // Le fault courant est maintenant autoritaire. L'anticipation est
            // opportuniste et ne change jamais son resultat : elle valide le
            // meme VMA Zero sous Mm et s'arrete au premier doute.
            drop(p);
            fault_cluster_after_zero(processus, fault_pml4, page, zero_id, zero_flags);
            stall_pf_phase(229, page);
            FaultOutcome::Resolved
        }
        PromesseBacking::Framebuffer { phys_base, mapping_start, phys_offset } => {
            let phys = phys_base.saturating_add(phys_offset)
                .saturating_add(page.saturating_sub(mapping_start));
            if p.space.map_foreign_accounted(
                page,
                phys,
                effective.drapeaux,
                crate::kernel::vmm::ResidentKind::Device,
            ) {
                note.pose(crate::kernel::fautes::Categorie::Materiel);
                FaultOutcome::Resolved
            } else {
                FaultOutcome::IoError
            }
        }
        PromesseBacking::SharedFile { node, mapping_start, file_offset, .. } => {
            let source = file_offset.saturating_add(page.saturating_sub(mapping_start));
            let numero = source / crate::kernel::vmm::PAGE_SIZE;
            drop(p);
            let lease = match crate::kernel::partage::page(node, numero) {
                Some(lease) => lease,
                None => return FaultOutcome::IoError,
            };
            let mut p = processus.mm.lock();
            if p.space.pml4() != fault_pml4 {
                return FaultOutcome::Retired;
            }
            if mapping_token(&p.promesses, page).as_ref() != Some(token) {
                return FaultOutcome::Retry;
            }
            if p.space.translate(page).is_some() {
                return FaultOutcome::Resolved;
            }
            if !p.space.map_foreign_accounted(
                page,
                lease.frame(),
                effective.drapeaux,
                crate::kernel::vmm::ResidentKind::Shared,
            ) {
                return FaultOutcome::IoError;
            }
            FAULTS_FILE.fetch_add(1, Ordering::Relaxed);
            note.pose(crate::kernel::fautes::Categorie::Partage);
            FaultOutcome::Resolved
        }
        PromesseBacking::File { .. } => {
            let regions = p.promesses.clone();
            let page_end = page + crate::kernel::vmm::PAGE_SIZE;
            let current_flags = effective.drapeaux;
            let mut clean_key = None;
            if effective.drapeaux & crate::kernel::vmm::PTE_WRITE == 0 {
                let covering: Vec<_> = regions.iter().filter(|region| {
                    matches!(region.backing, PromesseBacking::File { .. })
                        && region.chevauche(page, page_end)
                }).collect();
                if covering.len() == 1 {
                    if let PromesseBacking::File { node, mapping_start, file_offset, file_size } = effective.backing {
                        let data_end = mapping_start.saturating_add(file_size);
                        let offset = file_offset.saturating_add(page.saturating_sub(mapping_start));
                        if page >= mapping_start && page_end <= data_end
                            && offset % crate::kernel::vmm::PAGE_SIZE == 0
                            && crate::fs::backing::is_disk_backed(node)
                        {
                            if let Some(generation) = crate::fs::backing::generation(node) {
                                clean_key = Some(crate::kernel::clean_page_cache::Key { node, offset, generation });
                            }
                        }
                    }
                }
            }
            drop(p);

            if let Some(key) = clean_key {
                if let Some(frame) = crate::kernel::clean_page_cache::acquire(key) {
                    // No process-MM guard is held while readahead performs disk/cache work.
                    crate::kernel::readahead::observe_clean(key);
                    let mut mm = processus.mm.lock();
                    let outcome = if mm.space.pml4() != fault_pml4 {
                        FaultOutcome::Retired
                    } else if mapping_token(&mm.promesses, page).as_ref() != Some(token) {
                        FaultOutcome::Retry
                    } else if mm.space.translate(page).is_some() {
                        FaultOutcome::Resolved
                    } else if mm.space.map_foreign_accounted(
                        page,
                        frame,
                        current_flags,
                        crate::kernel::vmm::ResidentKind::FilePrivate,
                    ) {
                        mm.clean_pages.push(CleanPageMapping { virt: page, key });
                        FAULTS_FILE.fetch_add(1, Ordering::Relaxed);
                        note.pose(crate::kernel::fautes::Categorie::FichierPrive);
                        // The current mapping reference now owns `key`. Publish
                        // verified neighbours only after dropping Mm; their cache
                        // acquisition may perform disk I/O and must never extend
                        // this critical section.
                        drop(mm);
                        fault_cluster_after_clean(processus, fault_pml4, &regions, page);
                        return FaultOutcome::Resolved;
                    } else {
                        FaultOutcome::IoError
                    };
                    crate::kernel::clean_page_cache::release(key);
                    return outcome;
                }
            }

            // Build the complete private page off-MM and publish it only
            // after the range-local token has been revalidated. Mapping a zero
            // frame before I/O allowed concurrent mprotect to leave a present
            // but never-populated page behind.
            let mut page_data = [0u8; crate::kernel::vmm::PAGE_SIZE as usize];
            for region in regions {
                if !region.chevauche(page, page_end) { continue; }
                let (node, mapping_start, file_offset, file_size) = match region.backing {
                    PromesseBacking::File { node, mapping_start, file_offset, file_size } =>
                        (node, mapping_start, file_offset, file_size),
                    _ => continue,
                };
                let start = core::cmp::max(page, mapping_start);
                let end = core::cmp::min(page_end, mapping_start.saturating_add(file_size));
                if end <= start { continue; }
                let wanted = (end - start) as usize;
                let destination = (start - page) as usize;
                let source_offset = file_offset.saturating_add(start.saturating_sub(mapping_start));
                stall_pf_file_begin(source_offset);
                let got = crate::fs::backing::read_at(
                    node,
                    source_offset as usize,
                    &mut page_data[destination..destination + wanted],
                );
                stall_pf_file_done(got, wanted);
                if got != wanted { return FaultOutcome::IoError; }
            }

            let mut mm = processus.mm.lock();
            if mm.space.pml4() != fault_pml4 { return FaultOutcome::Retired; }
            if mapping_token(&mm.promesses, page).as_ref() != Some(token) {
                return FaultOutcome::Retry;
            }
            if mm.space.translate(page).is_some() { return FaultOutcome::Resolved; }
            if !mm.space.map_alloc_accounted(
                page,
                crate::kernel::vmm::PAGE_SIZE,
                effective.drapeaux,
                crate::kernel::vmm::ResidentKind::FilePrivate,
            ) {
                return FaultOutcome::IoError;
            }
            if !mm.space.write(page, &page_data) {
                let retirement = mm.space.prepare_unmap(page, crate::kernel::vmm::PAGE_SIZE);
                drop(mm);
                let depth = smp_lock::suspend_for_schedule();
                retirement.invalidation().execute();
                smp_lock::resume_after_schedule(depth);
                processus.mm.lock().space.finish_unmap(retirement);
                return FaultOutcome::IoError;
            }
            FAULTS_FILE.fetch_add(1, Ordering::Relaxed);
            note.pose(crate::kernel::fautes::Categorie::FichierPrive);
            FaultOutcome::Resolved
        }
    }
}

pub fn demand_fault_stats() -> (u64, u64) {
    (
        FAULTS_ZERO.load(Ordering::Relaxed),
        FAULTS_FILE.load(Ordering::Relaxed),
    )
}

pub fn demand_fault_waits() -> u64 {
    FAULT_WAITS.load(Ordering::Relaxed)
}

pub fn fault_outcome_stats() -> (u64, u64, u64, u64, u64) {
    (
        FAULT_RESOLVED.load(Ordering::Relaxed),
        FAULT_RETRY.load(Ordering::Relaxed),
        FAULT_INVALID.load(Ordering::Relaxed),
        FAULT_IO_ERROR.load(Ordering::Relaxed),
        FAULT_RETIRED.load(Ordering::Relaxed),
    )
}

pub fn fault_registry_stats() -> (u64, u64) {
    (
        FAULT_PAGES.lock().len() as u64,
        FAULT_REGISTRY_PEAK.load(Ordering::Relaxed),
    )
}

/// Diagnostic d'une faute que la Memory Fabric n'a pas pu servir.
pub fn log_fault_mapping(adresse: u64) {
    if !in_user_task() {
        return;
    }

    let process = current_process();
    let mut p = process.mm.lock();
    let page = adresse & !(crate::kernel::vmm::PAGE_SIZE - 1);
    let present = p.space.translate(page).is_some();
    let writable = p.space.writable(page);

    if let Some(region) = crate::kernel::vma::trouve(&p.promesses, page) {
        crate::println!(
            "[memfabric] FAULT_FATAL pid={} app={} addr={:#x} page={:#x} vma={:#x}..{:#x} backing={} flags={:#x} present={} writable={}",
            process.pid,
            process.metadata.lock().name,
            adresse,
            page,
            region.debut,
            region.fin,
            region.backing.label(),
            region.drapeaux,
            present,
            writable
        );
    } else {
        let (avant, apres) =
            crate::kernel::vma::voisines(&p.promesses, page);
        crate::println!(
            "[memfabric] FAULT_FATAL pid={} app={} addr={:#x} page={:#x} AUCUNE_VMA vmas={} present={} writable={}",
            process.pid,
            process.metadata.lock().name,
            adresse,
            page,
            p.promesses.len(),
            present,
            writable
        );
        if let Some(region) = avant {
            crate::println!(
                "[memfabric] vma precedente {:#x}..{:#x} backing={} flags={:#x}",
                region.debut,
                region.fin,
                region.backing.label(),
                region.drapeaux
            );
        }
        if let Some(region) = apres {
            crate::println!(
                "[memfabric] vma suivante {:#x}..{:#x} backing={} flags={:#x}",
                region.debut,
                region.fin,
                region.backing.label(),
                region.drapeaux
            );
        }
    }
}



