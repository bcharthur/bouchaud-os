fn release_one(cpu: usize) {
    // OWNER + DEPTH doivent changer atomiquement vis-a-vis d'une IRQ locale.
    let _irq = LocalIrqGuard::acquire();

    let etat = etat_charge(Ordering::Acquire);
    let depth = if etat.owner == token(cpu) { etat.depth } else { 0 };
    let owner = etat.owner;

    // Enregistrer AVANT les assertions : c'est cette transition-la qui explique
    // la violation, et une assertion qui panique n'y reviendrait jamais.
    enregistreur::note(
        enregistreur::RELEASE,
        cpu,
        owner,
        if depth > 1 { owner } else { FREE },
        depth,
        depth.saturating_sub(1),
        usize::MAX,
        token(cpu) as u64,
    );

    // Vider ICI, et pas seulement dans le `panic_handler`.
    //
    // Le handler y arrive desormais avant le releve riche, mais il passe quand
    // meme par l'arbitrage global, la VGA et `println!`. Or ce qu'on tient a cet
    // instant precis est la transition FAUTIVE elle-meme : c'est le moment ou
    // l'anneau vaut le plus cher, et le moment ou le noyau est le moins digne
    // de confiance. Le vidage est idempotent (l'anneau se gele au premier
    // appel), donc le handler n'en produira pas un second.
    if depth == 0 || owner != token(cpu) {
        crate::serial_println_brut!(
            "[BKL-FR] VIOLATION release cpu={} depth={} owner={} attendu={}",
            cpu, depth, owner, token(cpu),
        );
        vide_enregistreur();
    }

    debug_assert!(depth > 0, "smp_lock: release sans acquisition");
    debug_assert_eq!(
        owner,
        token(cpu),
        "smp_lock: release par un CPU non proprietaire"
    );

    if depth > 1 {
        remplace_profondeur_possedee(cpu, depth, depth - 1, Ordering::AcqRel)
            .expect("smp_lock: etat modifie pendant release reentrant");
        return;
    }

    // Fermer l'intervalle AVANT de publier FREE. Apres le CAS, un autre CPU
    // peut ouvrir son propre intervalle immediatement ; inverser ces deux
    // operations ferait se chevaucher deux tenues d'un verrou exclusif.
    probe_note_release(cpu, 1);
    remplace_profondeur_possedee(cpu, 1, 0, Ordering::SeqCst)
        .expect("smp_lock: etat modifie pendant release final");
    // Le CAS ci-dessus est SeqCst : c'est l'ordre total avec PARKED qui ferme
    // le reveil perdu. Voir wait_for_owner_change.
    wake_parked_waiters(cpu);
}
