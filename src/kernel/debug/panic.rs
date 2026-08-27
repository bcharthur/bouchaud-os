//! Handler de panique noyau.
//!
//! # Pourquoi il fait partie de l'arret global SMP
//!
//! Le mecanisme d'arret global (`arch::x86_64::idt`) ne couvrait que les
//! EXCEPTIONS : double faute, faute de page, #GP. Un `panic!` Rust -- donc
//! toute assertion du noyau, a commencer par celles du gros verrou -- n'y
//! entrait pas. Le CPU fautif imprimait `*** KERNEL PANIC ***` puis s'arretait
//! seul, et les trois autres continuaient : Ladybird tournait encore plusieurs
//! secondes apres la panique.
//!
//! Ce n'est pas un detail cosmetique. Une panique dit que l'etat du noyau n'est
//! plus celui qu'on croit ; laisser les autres coeurs continuer a s'en servir,
//! c'est laisser la corruption s'etendre PAR-DESSUS la trace qu'on est en train
//! d'ecrire, et rendre le diagnostic faux.
//!
//! # L'ordre, et sa raison
//!
//! 1. `cli` local. Le releve ne doit pas etre preempte, et surtout ce chemin ne
//!    prend AUCUN verrou : paniquer en tenant le BKL est le cas nominal, pas
//!    l'exception, donc le demander ici figerait la machine sans un mot.
//! 2. Arbitrage : le premier CPU arrive prend la panique. Les suivants -- et ce
//!    meme CPU s'il repanique dans le handler -- se taisent et s'arretent, pour
//!    que la sortie reste lisible au lieu d'entrelacer deux traces.
//! 3. L'enregistreur de vol du gros verrou, EN PREMIER, et par la sortie serie
//!    brute. Voir plus bas : c'est l'ordre qui a change, et il a change parce
//!    qu'on perdait la trace.
//! 4. Le releve de contexte riche.
//! 5. Puis SEULEMENT ENSUITE l'IPI d'arret. C'est le meme ordre que la double
//!    faute, et pour la meme raison : si les autres CPU s'arretaient avant, une
//!    faute pendant le releve laisserait la machine muette et figee. La fenetre
//!    ainsi laissee aux autres coeurs se compte en millisecondes, pas en
//!    secondes.
//! 6. `cli; hlt` definitif.
//!
//! # Pourquoi l'enregistreur passe AVANT le releve de contexte
//!
//! Sur les paniques Gate 1A, la sortie s'arretait net apres
//! `[FAULT] tss_rsp0=... gs_cpu_index=0` : `vide_enregistreur()` n'etait jamais
//! atteint. Le releve riche lit la table des taches, la pile noyau attendue et
//! la provenance du verrou -- c'est-a-dire, precisement, les structures dont la
//! panique vient de dire qu'elles ne sont plus fiables. Le faire passer avant
//! le vidage revenait a parier la trace entiere sur le morceau le plus fragile
//! du diagnostic.
//!
//! L'enregistreur, lui, ne lit qu'un anneau d'atomiques et n'ecrit que par
//! `serial_println_brut!` : COM1 en direct, sans le prefixe de journal qui
//! interroge l'horloge RTC, la charge CPU, la memoire et le disque. C'est le
//! chemin le plus court entre l'etat corrompu et le cable serie, donc c'est
//! celui qui doit s'executer en premier.
//!
//! Avec `panic = "abort"` il n'y a pas de deroulement de pile.

use core::panic::PanicInfo;
use crate::arch::x86_64::{idt, smp};
use crate::drivers::vga;
use crate::serial_println;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // Aucune IRQ pendant le releve, et aucun verrou pris : on panique
    // frequemment EN TENANT le gros verrou.
    unsafe { core::arch::asm!("cli", options(nomem, nostack)) };

    let cpu = smp::cpu_index();

    // Un second paniqueur -- autre CPU, ou ce CPU qui repanique dans le
    // handler -- n'a rien a ajouter : sa trace entrelacee rendrait les deux
    // illisibles, et sa recursion n'aurait pas de fond.
    if !idt::prends_la_panique(cpu) {
        idt::arret_definitif();
    }

    vga::set_color(vga::COLOR_RED);
    println!("");
    println!("*** KERNEL PANIC ***");
    println!("{}", info);
    vga::set_color(vga::COLOR_DEFAULT);

    serial_println!("");
    serial_println!("======== [KERNEL PANIC] ========");
    serial_println!("*** KERNEL PANIC *** cpu={}", cpu);
    serial_println!("{}", info);

    // D'ABORD la trace la moins fragile : l'anneau d'atomiques, en serie brute.
    crate::kernel::smp_lock::vide_enregistreur();

    // ENSUITE seulement le contexte riche. Les assertions du gros verrou
    // paniquent : leur contexte -- tache, pile noyau attendue, provenance du
    // verrou -- vaut autant ici que sur une exception. Mais il lit des
    // structures que la panique vient de declarer suspectes, donc il vient
    // apres. Pas de trame, donc pas de RSP a comparer : on le dit.
    idt::releve_contexte_courant(cpu, None);
    serial_println!("======== fin du releve ========");

    // Maintenant que le releve est ecrit, plus personne ne touche a rien.
    smp::arrete_les_autres_cpu();

    idt::arret_definitif();
}
