# Product M1.2 / ARCH P1 — qualification sans faux verts

Ce document est un **contrat de test**, pas une preuve de reussite. Le patch
modifie quatre fichiers Rust et fournit des outils. Il ne remplace pas Ladybird,
le GPU, le pilote xHCI, le VFS, le scheduler ou le stockage.

## P0 — Souris et diagnostic scheduler
- `diagnostic_stall.rs`: limite a une fois toutes les 30 s les dumps exhaustifs
  causes **uniquement** par `LatenceHid`. Fatal, manuel et reseau restent
  immédiats et intégraux. `[M12-HID-DIAG]` affiche demandes, dumps, quotas.
- `window_manager.rs`: une ligne `[M12-GUI-COMPOSITION]` toutes les ~10 s, avec
  nombres cumules exacts des appels.
- **Non prouve** : amelioration reelle de la latence HID; comparer par minute et
  garder les preuves xHCI STALL.

## P1 — horloge et extinction
- `timer.rs`: classe la comparaison TSC/ticks en `degraded` (ecart >10%),
  `candidate` (<=10%) ou `unknown`. Un resultat `candidate` n'est **pas** une
  preuve d'horloge fiable. Pas de correction automatique d'une horloge en vol.
- `power.rs`: jalons `before_finish`, `after_finish`, `before_emulator_ports`,
  `BOUCHAUD_M12_ACPI_S5_ATTEMPT`, `BOUCHAUD_M12_ACPI_S5_RETURNED`.
- Sans `-DebugExit`: demander Eteindre, QEMU doit se fermer tout seul. Les
  marqueurs servent au diagnostic. Une fermeture manuelle n'est pas un succes.
- Avec `-DebugExit`: le code QEMU n'est PAS une preuve ACPI; il prouve seulement
  que l'appel de terminaison atteint le port QEMU.

## P2 — Notes durables et tests ABI/IPC negatifs
- Le test QEMU `-PersistLab` utilise un second disque virtuel explicitement
  inscriptible. Apres sauvegarde Notes, il faut fermer et relancer completement
  QEMU sur le meme volume, puis relire le texte. Sans cela: **non qualifie**.
- Les tests ARCH P1 positifs ring3 SMP4 sont passes lors de la session
  precedente. Le cas **negatif** `CHANNEL_RECV` avec copie utilisateur fautive,
  remise en file FIFO, preservation et absence de duplication de handles,
  reste **non execute**. Aucune assertion statique ne le remplace.

## Baseline QEMU 2026-10-16
- 25 507 lignes, 1074 secondes, 5 736 591 octets.
- 174 `HID_LATENCY_SPIKE`, 174 dumps complets, 9319 `[SCHED-TACHE]`,
  2069 `GUI_COMPOSITION_NAVIGATEUR`, 2 veritables STALL HID.
- 1 vCPU TCG; consommation CPU elevee, environnement non representatif KVM.
- FADT S5 interpretee `etat=pret`; pas d'ecriture ACPI S5 enregistree.
- Sauvegarde Notes `durable=0`, QEMU snapshot un seul disque.

## Recette de qualification

1. Verifier le dry-run et l'empreinte avant toute application.
2. Compiler UEFI (features `uefi-boot,reference-bringup,reference-desktop`).
3. Conserver SHA baseline; reconstruire volontairement `IMAGE-TRIGKEY.ps1`.
4. Rejouer meme image logique, meme CPU QEMU, resolution, acceleration, sites,
   navigation et duree, comparer via `COMPARE-M12.ps1`.
5. Analyser les preuves nouvelles dans l'ordre du chemin d'extinction.
6. Tester Notes avec disque secondaire deux boots.
7. Integrer une sonde ring3 negative IPC distincte de la sonde fonctionnelle.

## Chantiers ouverts (aucun vert invente)

VFS/page-cache coherent, volume durable, IRQ xHCI root-cause, base de temps SMP
fiable, GPU/Compositor zero-copy et damage, Ladybird logs dupliques a la
source, negative IPC runtime, preuve electrique S5 Trigkey. Cette archive ne
prouve aucun de ces chantiers par elle-meme.
