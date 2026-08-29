pub struct KernelGuard {
    cpu: usize,
    active: bool,
}

impl KernelGuard {
    /// Identite d'un garde pour l'enregistreur de vol : son ADRESSE sur la pile
    /// noyau. Deux gardes imbriques d'une meme tache ont le meme `cpu` et la
    /// meme tache ; seule leur adresse les distingue, et c'est justement ce
    /// qu'il faut pour savoir lequel a ete relache deux fois.
    #[inline]
    fn identite(&self) -> u64 {
        self as *const KernelGuard as u64
    }
}

impl Drop for KernelGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }

        // BOUCHAUD_SMP_NG2_BKL_MIGRATION_HOTFIX_V1
        //
        // Un KernelGuard vit sur la pile noyau de la tache. Depuis NG2 cette
        // pile peut reprendre sur un autre CPU apres suspend_for_schedule().
        // resume_after_schedule() restaure alors DEPTH/OWNER sur le NOUVEAU
        // CPU. Le champ `self.cpu` ne represente plus le proprietaire actuel:
        // il indique seulement le CPU sur lequel le guard a ete cree.
        //
        // Liberer `self.cpu` apres une migration donne DEPTH[ancien_cpu] == 0
        // et provoque exactement: "smp_lock: release sans acquisition".
        // La profondeur BKL suit la continuation; son Drop doit donc liberer le
        // CPU physique/logique sur lequel cette continuation s'execute maintenant.
        // Masquer AVANT de lire l'index : sans cela une IRQ pourrait commuter
        // entre la lecture et la liberation, et `release_one` rendrait le
        // verrou au nom d'un CPU qui ne le detient pas. Le garde interne de
        // `release_one` s'imbrique sans dommage : chacun restaure l'etat qu'il
        // a trouve.
        let _irq = LocalIrqGuard::acquire();
        let release_cpu = cpu();
        {
            let owner = OWNER.load(Ordering::Relaxed);
            let depth = DEPTH[release_cpu].load(Ordering::Relaxed);
            enregistreur::note(
                enregistreur::GUARD_DROP,
                release_cpu,
                owner,
                owner,
                depth,
                depth,
                self.cpu,
                self.identite(),
            );
        }
        release_one(release_cpu);
    }
}
