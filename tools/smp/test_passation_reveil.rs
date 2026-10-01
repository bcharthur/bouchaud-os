//! Passation et reveil : la tache bloquee qu'on reveille pendant qu'elle
//! quitte son coeur n'est jamais perdue.
//!
//! BOUCHAUD_PASSATION_REVEIL_ORDONNES_V1
//!
//! # Le defaut
//!
//! Campagne Scheduler NG phase 1, SMP4 (1 demarrage sur 12) : une tache
//! `Ready`, hors de tout coeur, dans aucune file -- `[SCHED-ORPHELINE] tid=115
//! ... prete hors de toute file DEUX passes de suite : elle ne sera jamais
//! elue` -- et le banc fige sur la lecture du tube que cette tache devait
//! remplir.
//!
//! Deux cotes se croisent quand une tache bloquee est reveillee pendant sa
//! commutation de sortie :
//!
//!   passation (coeur sortant)   on_cpu = -1 ; switching_out = false ;
//!                               lit l'etat -> Ready ? republie
//!   reveilleur (autre coeur)    CAS Blocked -> Ready ;
//!                               lit switching_out / on_cpu -> libre ? publie
//!
//! C'est le motif « store buffer » : chaque cote ECRIT puis LIT un autre
//! emplacement. En x86-TSO, la lecture de la passation peut passer devant ses
//! propres ecritures (encore dans le tampon d'ecriture du coeur) : elle lit
//! `Blocked`, le reveilleur lit `switching_out == true`, et PERSONNE ne publie.
//! Les ecritures `Release` et lectures `Acquire` n'interdisent pas ce
//! reordonnancement ; une barriere `SeqCst` entre l'ecriture et la lecture,
//! des deux cotes, l'interdit.
//!
//! # Ce que ce test prouve
//!
//! 1. Un modele EXHAUSTIF de x86-TSO (tampon d'ecriture FIFO par coeur, vidage
//!    non deterministe, instructions verrouillees et barrieres qui vident le
//!    tampon) explore toutes les executions des deux protocoles : l'ancien a
//!    une execution qui perd la tache, le nouveau n'en a aucune.
//! 2. Le meme protocole sur de vrais fils de l'hote, avec les MEMES
//!    ordonnancements que le noyau : le nouveau ne perd jamais la tache ;
//!    l'ancien est mesure et publie (sa perte depend du materiel, elle n'est
//!    donc pas exigee -- le modele, lui, est deterministe).
//!
//! Lance par `tools/ci/run_host_tests.sh`.

use std::collections::HashSet;
use std::sync::atomic::{fence, AtomicBool, AtomicI8, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Barrier};

// ---------------------------------------------------------------------------
// 1. Modele x86-TSO exhaustif
// ---------------------------------------------------------------------------

const ETAT: usize = 0; // 0 = Ready, 1 = Blocked
const SWITCHING: usize = 1; // 1 = vrai
const ON_CPU: usize = 2; // 0 = sur le coeur, 255 = -1
const READY: u8 = 0;
const BLOCKED: u8 = 1;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Op {
    /// Ecriture ordinaire : entre dans le tampon d'ecriture du coeur.
    Ecrit(usize, u8),
    /// Barriere : attend que le tampon soit vide.
    Barriere,
    /// CAS verrouille : tampon vide, puis lecture-ecriture atomique en
    /// memoire. Resultat dans le registre.
    Cas(usize, u8, u8),
    /// Lecture : dans le tampon du coeur d'abord, en memoire sinon.
    Lit(usize),
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Fil {
    pc: usize,
    tampon: Vec<(usize, u8)>,
    lus: Vec<u8>,
    cas_ok: bool,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Machine {
    memoire: [u8; 3],
    fils: [Fil; 2],
}

fn lit(fil: &Fil, memoire: &[u8; 3], adresse: usize) -> u8 {
    fil.tampon
        .iter()
        .rev()
        .find(|(a, _)| *a == adresse)
        .map(|(_, v)| *v)
        .unwrap_or(memoire[adresse])
}

fn programmes(nouveau: bool) -> [Vec<Op>; 2] {
    let mut passation = vec![Op::Ecrit(ON_CPU, 255), Op::Ecrit(SWITCHING, 0)];
    if nouveau {
        passation.push(Op::Barriere);
    }
    passation.push(Op::Lit(ETAT));
    let mut reveil = vec![Op::Cas(ETAT, BLOCKED, READY)];
    if nouveau {
        reveil.push(Op::Barriere);
    }
    reveil.push(Op::Lit(SWITCHING));
    reveil.push(Op::Lit(ON_CPU));
    [passation, reveil]
}

/// Publications d'une execution terminee : la passation publie si elle a lu
/// Ready ; le reveilleur publie s'il a gagne le CAS et vu la tache libre.
fn publications(m: &Machine) -> u32 {
    let passation = (*m.fils[0].lus.last().unwrap() == READY) as u32;
    let r = &m.fils[1].lus;
    let libre = r[0] == 0 && r[1] == 255;
    let reveil = (m.fils[1].cas_ok && libre) as u32;
    passation + reveil
}

#[derive(Default, Debug)]
struct Bilan {
    executions: usize,
    perdues: usize,
    doubles: usize,
}

fn explore(nouveau: bool) -> Bilan {
    let progs = programmes(nouveau);
    let vide = Fil { pc: 0, tampon: vec![], lus: vec![], cas_ok: false };
    // Etat initial : tache bloquee, encore sur son coeur, en cours de sortie.
    let depart = Machine { memoire: [BLOCKED, 1, 0], fils: [vide.clone(), vide] };
    let mut vus = HashSet::new();
    let mut pile = vec![depart];
    let mut bilan = Bilan::default();
    while let Some(m) = pile.pop() {
        if !vus.insert(m.clone()) {
            continue;
        }
        let mut suivants = Vec::new();
        for i in 0..2 {
            // Vidage d'une ecriture du tampon vers la memoire.
            if !m.fils[i].tampon.is_empty() {
                let mut s = m.clone();
                let (a, v) = s.fils[i].tampon.remove(0);
                s.memoire[a] = v;
                suivants.push(s);
            }
            let Some(op) = progs[i].get(m.fils[i].pc).copied() else { continue };
            let mut s = m.clone();
            let fil = &mut s.fils[i];
            match op {
                Op::Ecrit(a, v) => {
                    fil.tampon.push((a, v));
                    fil.pc += 1;
                }
                Op::Barriere => {
                    if !fil.tampon.is_empty() {
                        continue;
                    }
                    fil.pc += 1;
                }
                Op::Cas(a, attendu, nouveau_v) => {
                    if !fil.tampon.is_empty() {
                        continue;
                    }
                    let actuel = s.memoire[a];
                    let fil = &mut s.fils[i];
                    fil.cas_ok = actuel == attendu;
                    fil.pc += 1;
                    if actuel == attendu {
                        s.memoire[a] = nouveau_v;
                    }
                }
                Op::Lit(a) => {
                    let v = lit(fil, &s.memoire, a);
                    let fil = &mut s.fils[i];
                    fil.lus.push(v);
                    fil.pc += 1;
                }
            }
            suivants.push(s);
        }
        if suivants.is_empty() {
            bilan.executions += 1;
            match publications(&m) {
                0 => bilan.perdues += 1,
                2 => bilan.doubles += 1,
                _ => {}
            }
        }
        pile.extend(suivants);
    }
    bilan
}

#[test]
fn modele_tso_l_ancien_protocole_perd_la_tache() {
    let b = explore(false);
    assert!(b.executions > 0);
    assert!(b.perdues > 0, "ancien protocole : aucune execution perdante trouvee {b:?}");
}

#[test]
fn modele_tso_le_nouveau_protocole_ne_perd_jamais() {
    let b = explore(true);
    assert!(b.executions > 0);
    assert_eq!(b.perdues, 0, "nouveau protocole : {b:?}");
}

// ---------------------------------------------------------------------------
// 2. Vrais fils de l'hote
// ---------------------------------------------------------------------------

fn course(nouveau: bool, tours: u32) -> (u32, u32) {
    let etat = Arc::new(AtomicU8::new(BLOCKED));
    let switching = Arc::new(AtomicBool::new(true));
    let on_cpu = Arc::new(AtomicI8::new(0));
    let publiees = Arc::new(AtomicU32::new(0));
    let depart = Arc::new(Barrier::new(2));
    let fin = Arc::new(Barrier::new(3));
    let perdues = Arc::new(AtomicU32::new(0));

    let passation = {
        let (etat, switching, on_cpu, publiees, depart, fin) =
            (etat.clone(), switching.clone(), on_cpu.clone(), publiees.clone(), depart.clone(), fin.clone());
        std::thread::spawn(move || {
            for _ in 0..tours {
                depart.wait();
                on_cpu.store(-1, Ordering::Release);
                switching.store(false, Ordering::Release);
                if nouveau {
                    fence(Ordering::SeqCst);
                }
                if etat.load(Ordering::Acquire) == READY {
                    publiees.fetch_add(1, Ordering::Relaxed);
                }
                fin.wait();
                fin.wait();
            }
        })
    };
    let reveil = {
        let (etat, switching, on_cpu, publiees, depart, fin) =
            (etat.clone(), switching.clone(), on_cpu.clone(), publiees.clone(), depart.clone(), fin.clone());
        std::thread::spawn(move || {
            for _ in 0..tours {
                depart.wait();
                let gagne = etat
                    .compare_exchange(BLOCKED, READY, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok();
                if nouveau {
                    fence(Ordering::SeqCst);
                }
                if gagne && !switching.load(Ordering::Acquire) && on_cpu.load(Ordering::Acquire) < 0 {
                    publiees.fetch_add(1, Ordering::Relaxed);
                }
                fin.wait();
                fin.wait();
            }
        })
    };
    let mut doubles = 0;
    for _ in 0..tours {
        fin.wait();
        match publiees.swap(0, Ordering::Relaxed) {
            0 => {
                perdues.fetch_add(1, Ordering::Relaxed);
            }
            2 => doubles += 1,
            _ => {}
        }
        etat.store(BLOCKED, Ordering::Relaxed);
        switching.store(true, Ordering::Relaxed);
        on_cpu.store(0, Ordering::Relaxed);
        fin.wait();
    }
    passation.join().unwrap();
    reveil.join().unwrap();
    (perdues.load(Ordering::Relaxed), doubles)
}

#[test]
fn vrais_fils_le_nouveau_protocole_ne_perd_jamais() {
    let (perdues, doubles) = course(true, 50_000);
    println!("nouveau : perdues={perdues} doubles={doubles} sur 50000");
    assert_eq!(perdues, 0);
}

#[test]
fn vrais_fils_l_ancien_protocole_est_mesure() {
    let (perdues, doubles) = course(false, 50_000);
    // Publie, pas exige : la frequence depend du coeur hote.
    println!("ancien : perdues={perdues} doubles={doubles} sur 50000");
}
