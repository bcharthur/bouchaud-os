// Suspension temporaire du BKL avant blocage/context switch.

/// Libere completement le BKL avant un switch de contexte et rend la profondeur
/// a restaurer lorsque cette pile noyau reprendra.
pub fn suspend_for_schedule() -> usize {
    // C'etait la race observee : auparavant DEPTH passait a 0 avant OWNER,
    // avec IRQ encore actives. Le PIT pouvait alors entrer reentrant, puis
    // liberer OWNER avant notre assertion.
    //
    // L'index du CPU se lit APRES le masquage, pour la meme raison qu'ailleurs
    // dans ce fichier : lu avant, une commutation entre les deux ferait
    // suspendre au nom d'un coeur qui ne detient rien.
    let _irq = LocalIrqGuard::acquire();
    let cpu = cpu();
    let mine = token(cpu);

    let etat = etat_charge(Ordering::Acquire);
    let depth = if etat.owner == mine { etat.depth } else { 0 };
    note_schedule_suspend(cpu, depth);
    if depth == 0 {
        return 0;
    }

    #[cfg(debug_assertions)]

    debug_assert_eq!(
        etat.owner,
        mine,
        "smp_lock: suspend sans ownership"
    );

    enregistreur::note(
        enregistreur::SUSPEND, cpu, mine, FREE, depth, 0, usize::MAX, depth as u64,
    );
    // Comme dans release_one, la comptabilite doit etre fermee avant que FREE
    // soit visible : un acquereur distant peut repartir des le CAS suivant.
    probe_note_release(cpu, 2);
    remplace_profondeur_possedee(cpu, depth, 0, Ordering::SeqCst)
        .expect("smp_lock: etat modifie pendant suspend_for_schedule");
    note_schedule_owner_released(cpu, depth);
    // Un changement de contexte libere le verrou aussi reellement qu'un Drop :
    // l'oublier ici laisserait dormir un CPU jusqu'a la prochaine liberation
    // ordinaire, qui peut ne jamais venir si c'est lui qui devait la produire.
    wake_parked_waiters(cpu);
    depth
}
