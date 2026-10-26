# P13 — passation corrective du 26 octobre 2026

**Statut : P13 NON VALIDÉ.** Base exacte : SHA `60803b8ed7bac84becd769c9492c60ed70df3638`. Cette branche est une **branche de preuves et de correction du rapport**, pas un correctif scheduler qualifié. Ne pas fusionner dans main.

## Matériel de référence

- Le mainteneur détient `BOUCHAUD-P13-PREUVES-ET-SOURCES-20261026.zip` (2 564 fichiers, sept journaux complets, sources et diffs) et `P13-CARTOGRAPHIE-CLAUDE-V4.tar.gz`. **Ces archives ne sont pas stockées sur cette branche** ; il faut les obtenir auprès du mainteneur pour l'attribution détaillée des incidents.
- Rapport réanalysé par le parseur officiel P13 : `RAPPORT_CLAUDE_V4.md`, extrait ci-dessous.
- Six des sept campagnes ont une vérification stricte P13 exécutée et négative. SMP2 est un diagnostic et non un résultat strict dans le qualificateur existant.
- 56 bras au total (8 par campagne), 168 lignes instrumentées `P13_PIRE_ECHANTILLON` (24 par campagne), un marqueur final par campagne. Aucune panique constatée.

## Synthèse des sept campagnes

| Variante / SMP | Politique | P1 | P2 | P3 | P4 | P5 |
|---|---|---|---|---|---|---|
| candidat / 1 | NON_CONVAINCANT | OK | ECHEC | OK | OK | OK |
| candidat / 2 | NON_CONVAINCANT, diagnostic | OK | ECHEC | NQ | OK | OK |
| candidat / 4 | NON_CONCLUANT 7/8 | NQ | NQ | NQ | OK* | OK* |
| avance OFF / 4 | NON_CONVAINCANT | OK | ECHEC | NQ | OK | OK |
| cible OFF / 1 | NON_CONVAINCANT | OK | ECHEC | OK | OK | OK |
| cible OFF / 4 | NON_CONCLUANT 7/8 | NQ | NQ | NQ | OK* | OK* |
| idle-only / 4 | NON_CONCLUANT 7/8 | NQ | NQ | NQ | OK* | OK* |

*OK sur les seuls bras admissibles, ne vaut **jamais** validation globale 8/8. Les paires strictes conservent leur quorum. La présente branche ajoute le marqueur explicite `AB_COUVERTURE_NON_EXHAUSTIVE` sans changer les seuils, les verdicts ou les codes retour.

### Anomalies prouvées

**P2 récurrent, y compris SMP1** : candidat SMP1 p95 interactif ON **44,5 µs** / OFF **20 µs** ; cible OFF SMP1 ON **24 µs** / OFF **18 µs** (encore rouge). Candidat SMP2 ON **70,5 µs** / OFF **28,5 µs**. Variante avance OFF SMP4 ON **72,5 µs** / OFF **42 µs**. La suppression seule de `avance_pour_sommeil_interactif()` ne résout pas P2. La suppression de `cible_occupee()` améliore P2 SMP4 dans certains essais mais ne qualifie pas SMP1 ni la campagne SMP4.

**P4 sporadique et non contaminé** : variante idle-only SMP4, tour 3 ON, pire interactif **402 µs**, supérieur à pire normal **147 µs** + pire repos **119 µs** = **266 µs** de **136 µs**. Échantillon instrumenté 171 : `pret_ns=83491`, `contamine=0`, `non_apparies=0`. La fraction du retard qui dépasse l'attente Ready→élu n'est **pas encore attribuée causalement**. Ne pas réduire le seuil ni écarter l'échantillon.

**P3** : souvent NON_QUALIFIANT parce que la charge ne produit aucun effet mesurable au regard du repos. Ne jamais le transformer en QUALIFIE_OK artificiel.

## Code à examiner avant toute correction Rust

1. `src/arch/x86_64/smp.rs` : `cible_occupee`, `avance_pour_sommeil_interactif`, `arm_local_scheduler_timer`, `rearme_reste_du_quantum`, `programme_deadline`, `tir_anticipe`. Vérifier l'invariant QUANTUM_TSC / ARME_TSC / ETAT_DEADLINE / ARME_OCCUPE, les interruptions anticipées et la nécessité des écritures du MSR.
2. `src/arch/x86_64/idt/reschedule.rs` : chemin timer/IPI `note_entree_vecteur_local` → `tir_anticipe` → `sert_echeances_au_vecteur_local` → réarmement → élection/préemption. Le résidu SMP1 impose d'examiner le coût par IRQ, pas seulement le parallélisme.
3. `src/kernel/process/thread/sommeil.rs` : `PROCHAINE_INTERACTIVE`, `borne_interactive_a_jour`, `wake_sleepers`, `publie_depuis_balayage` ; revue des publications avec `fetch_min`, recalculs et migrations.
4. `src/kernel/process/thread/creation.rs`, `ordonnancement.rs`, `commutation.rs`, `preemption.rs` : `publish_ready`, IPI ciblés, élection, passation SMP. Attribuer les cas P4 à des événements corrélés au **même** sommeil.
5. Vérifier en complément les hypothèses C5/C6/C11 sur leurs preuves propres ; les sept campagnes de réveil précis **ne** qualifient **pas** à elles seules toute la convergence P13.

## Travail accompli dans cette branche

- `tools/ci/analyse_ab_reveil_precis.py` affiche désormais `AB_COUVERTURE_NON_EXHAUSTIVE bras_valides=7/8` pour éviter qu'un `P4=QUALIFIE_OK` sur un sous-ensemble soit lu comme une qualification globale. La logique de décision est inchangée.
- `tools/ci/reliability/test_ab_p13_couverture.py` couvre 7/8 et 8/8.
- Tests exécutés sur l'archive exportée en Python 3 : compilation Python OK ; **20 tests passés** (qualification stricte, campagnes et couverture) avec `PYTHONPATH=tools/ci/reliability`. Les journaux réels candidat SMP4 et idle-only SMP4 déclenchent le nouveau marqueur 7/8 ; avance OFF 8/8 ne le déclenche pas.
- **Aucun Rust n'a été compilé et aucun nouveau test KVM n'a été exécuté ici.** Ne pas prétendre que le scheduler est corrigé.

## Ordre d'exécution attendu pour Claude

Implémenter une correction précise et réversible du scheduler, en conservant la préemption, les protections mémoire, le quantum et tous les seuils ; compiler et effectuer les tests hôte pertinents ; KVM SMP1/SMP4 (SMP2 en diagnostic) avec analyse stricte du même bootimage ; corriger chaque dépassement P4 ; puis geler un SHA et exécuter les campagnes de convergence requises. **Un bras non qualifié ne vaut jamais un bras vert.**

*Le ZIP intégral des sources/journaux se trouve dans la conversation de remise d'Arthur et en copie sur son poste Windows ; il n'a pas été envoyé sur GitHub.*
## Provenance de la sonde instrumentée

Le lanceur V3 exécutait `run_ab_reveil_precis.sh` **depuis le worktree `bouchaud-p13-sonde-pire-20261026`**, tandis que chaque `bootimage` provenait du worktree de la variante testée. Ainsi les 24 marqueurs `P13_PIRE_ECHANTILLON` par campagne proviennent de **la sonde userspace instrumentée**, sans impliquer que `tools/userland/ordonnanceur-probe.c` soit modifié dans le HEAD du candidat. Garder les deux identités distinctes pour toute reproduction ; reconstruire le bootimage Rust après chaque changement de noyau. Le contrôle statique `STATIC_CHECK ... True` du V3 est une vérification de présence textuelle, **pas** une preuve d'absence de course SMP.
