use std::sync::atomic::{AtomicBool, AtomicI8, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

const LIBRE: i8 = -1;

struct TacheModele {
    on_cpu: AtomicI8,
    sortante: AtomicBool,
    prete: AtomicBool,
    en_file: AtomicBool,
}

impl TacheModele {
    fn neuve() -> Self {
        Self {
            on_cpu: AtomicI8::new(LIBRE),
            sortante: AtomicBool::new(false),
            prete: AtomicBool::new(true),
            en_file: AtomicBool::new(false),
        }
    }

    fn revendique(&self, cpu: i8) -> bool {
        self.on_cpu
            .compare_exchange(LIBRE, cpu, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn prepare_sortie(&self, cpu: i8) {
        assert_eq!(self.on_cpu.load(Ordering::Acquire), cpu);
        assert!(!self.sortante.swap(true, Ordering::AcqRel));
    }

    fn complete_sortie(&self, cpu: i8, pile_abandonnee: bool) -> Result<(), &'static str> {
        if !pile_abandonnee {
            return Err("pile encore active");
        }
        assert_eq!(self.on_cpu.load(Ordering::Acquire), cpu);
        assert!(self.sortante.swap(false, Ordering::AcqRel));
        self.on_cpu.store(LIBRE, Ordering::Release);
        if self.prete.load(Ordering::Acquire) {
            self.en_file.store(true, Ordering::Release);
        }
        Ok(())
    }
}

#[test]
fn un_seul_cpu_revendique_une_tache() {
    let tache = Arc::new(TacheModele::neuve());
    let depart = Arc::new(Barrier::new(8));
    let gagnants = Arc::new(AtomicUsize::new(0));
    let mut fils = Vec::new();

    for cpu in 0..8i8 {
        let tache = Arc::clone(&tache);
        let depart = Arc::clone(&depart);
        let gagnants = Arc::clone(&gagnants);
        fils.push(thread::spawn(move || {
            depart.wait();
            if tache.revendique(cpu) {
                gagnants.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    for fil in fils {
        fil.join().unwrap();
    }
    assert_eq!(gagnants.load(Ordering::Relaxed), 1);
}

#[test]
fn la_porte_locale_refuse_la_reentree_irq() {
    let porte = AtomicBool::new(false);
    assert!(porte
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok());
    assert!(porte
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err());
    assert!(porte.swap(false, Ordering::AcqRel));
    assert!(porte
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok());
}

#[test]
fn la_sortante_n_est_publiee_qu_apres_abandon_de_pile() {
    let tache = TacheModele::neuve();
    assert!(tache.revendique(2));
    tache.prepare_sortie(2);
    assert_eq!(tache.complete_sortie(2, false), Err("pile encore active"));
    assert_eq!(tache.on_cpu.load(Ordering::Acquire), 2);
    assert!(!tache.en_file.load(Ordering::Acquire));

    tache.complete_sortie(2, true).unwrap();
    assert_eq!(tache.on_cpu.load(Ordering::Acquire), LIBRE);
    assert!(tache.en_file.load(Ordering::Acquire));
}

#[test]
fn un_reveil_pendant_la_passation_n_est_pas_perdu() {
    let tache = TacheModele::neuve();
    assert!(tache.revendique(1));
    tache.prete.store(false, Ordering::Release);
    tache.prepare_sortie(1);

    // Le reveilleur rend la tache prete, mais ne peut pas la publier tant
    // qu'elle possede encore physiquement la pile du CPU 1.
    tache.prete.store(true, Ordering::Release);
    assert!(!tache.en_file.load(Ordering::Acquire));

    tache.complete_sortie(1, true).unwrap();
    assert!(tache.en_file.load(Ordering::Acquire));
}

#[test]
fn le_coeur_du_scheduler_observe_toujours_depth_zero() {
    fn appel_legacy(profondeur: usize, observee: &AtomicUsize) -> usize {
        let suspendue = profondeur;
        observee.store(0, Ordering::Release);
        suspendue
    }

    let observee = AtomicUsize::new(usize::MAX);
    let rendue = appel_legacy(3, &observee);
    assert_eq!(observee.load(Ordering::Acquire), 0);
    assert_eq!(rendue, 3);
}

#[test]
fn une_sortie_definitive_abandonne_sa_profondeur_legacy() {
    fn abandonne_avant_switch(profondeur: &AtomicUsize) -> usize {
        profondeur.swap(0, Ordering::AcqRel)
    }

    // `exit/exit_group` entre encore par le dispatcher legacy avec un garde
    // vivant sur la pile. Cette pile ne reviendra jamais : il n'existe donc
    // aucun Drop futur auquel confier la liberation.
    let profondeur = AtomicUsize::new(1);
    let abandonnee = abandonne_avant_switch(&profondeur);
    assert_eq!(abandonnee, 1);
    assert_eq!(profondeur.load(Ordering::Acquire), 0);

    // Le meme helper reste sans effet pour une retraite deja detachee.
    assert_eq!(abandonne_avant_switch(&profondeur), 0);
    assert_eq!(profondeur.load(Ordering::Acquire), 0);
}

#[test]
fn une_sortie_definitive_ouvre_la_porte_avant_le_switch() {
    let porte = AtomicBool::new(false);
    let profondeur = AtomicUsize::new(1);

    // Ordre impose par le chemin reel : abandon du BKL, ouverture de la porte,
    // puis seulement election et commutation.
    profondeur.store(0, Ordering::Release);
    assert!(porte
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok());
    assert_eq!(profondeur.load(Ordering::Acquire), 0);
    assert!(porte.load(Ordering::Acquire));

    // Sans candidat la porte est rendue. Avec candidat elle reste vraie et la
    // continuation entrante la rend apres le changement physique de pile.
    assert!(porte.swap(false, Ordering::AcqRel));
}

#[test]
fn stress_revendication_recyclage_sans_double_execution() {
    for _ in 0..500 {
        let tache = Arc::new(TacheModele::neuve());
        let depart = Arc::new(Barrier::new(4));
        let gagnants = Arc::new(AtomicUsize::new(0));
        let mut fils = Vec::new();
        for cpu in 0..4i8 {
            let tache = Arc::clone(&tache);
            let depart = Arc::clone(&depart);
            let gagnants = Arc::clone(&gagnants);
            fils.push(thread::spawn(move || {
                depart.wait();
                if tache.revendique(cpu) {
                    gagnants.fetch_add(1, Ordering::Relaxed);
                }
            }));
        }
        for fil in fils {
            fil.join().unwrap();
        }
        assert_eq!(gagnants.load(Ordering::Relaxed), 1);
    }
}
