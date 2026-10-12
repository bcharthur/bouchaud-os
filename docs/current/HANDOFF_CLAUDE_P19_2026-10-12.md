# Bouchaud OS — handoff Claude P19 (12 octobre 2026)

> Document de reprise. **Ne pas traiter ce checkpoint comme une release validée.**

## 0. Lire d'abord

1. Ce fichier, puis `BOUCHAUD_AI_CONTEXT.md`, `docs/current/README.md` et `docs/current/HANDOFF_CODEX_2026-10-08.md`.
2. `git status --short`, `git branch --show-current`, `git rev-parse HEAD`, `git log --oneline -10`.
3. Examiner `git show --stat HEAD~1`, puis les diffs introduits par le checkpoint P19.
4. Confirmer les marqueurs et validations **sur le HEAD de reprise**, pas uniquement sur les journaux anciens.

## 1. Source de vérité Git

- Repo : `bcharthur/bouchaud-os`.
- Branche distante de travail : `claude/ladybird-observability-performance`. Ne jamais pousser sur `main`.
- Base avant P19 local : `788c5446b85a7fe131538eb37c37ba839ac42748`.
- Commit checkpoint des sources : `8e1bdf975c76c80a1b5999334cc54ddd9d0a6f50`.
- Le dépôt de test Windows était en **HEAD détaché** au moment du travail local ; c'est volontaire.
- Commits P19 poussés **sans force-push, sans rebase et sans reset**.
- Ne pas prendre les anciennes mentions `3658af0`, `952e95d` ou `3262af2` pour le HEAD courant.

## 2. Changements P19 inclus dans le checkpoint

- `.gitignore`
- `Cargo.toml`
- `src/arch/x86_64/idt/exceptions.rs`
- `src/gui/widgets.rs`
- `src/gui/window_manager.rs`
- `src/kernel/debug/blackbox.rs`
- `src/kernel/process/thread/faute_cluster.rs`
- `tools/ladybird/browser-upstream.sh`
- `tools/ladybird/chrome/BouchaudChargement.h`
- `tools/ladybird/chrome/BouchaudChrome.h`
- `tools/ladybird/fetch.sh`
- `tools/ladybird/prepare-m9-diagnostics.py`
- `tools/ladybird/ui-bouchaud/BrowserWindow.cpp`

Vérifier précisément dans le diff que ces changements incluent : Crash Guard du noyau, correctifs P19 mémoire, instrumentation, adaptation locale de la chaîne Ladybird, nouveau chrome et machine à états par onglet.
**Important** : le patch Rust ONE-LOADER qui retire la seconde barre côté noyau a été proposé, mais **son application locale n'était pas confirmée au moment de ce handoff**. Inspecter `src/gui/widgets.rs` et `src/gui/window_manager.rs` avant de conclure.

## 3. Preuves et limites observées

- P19 `Candidate` QEMU TCG : `MM-ZERO-CLUSTER max_batch=8` confirmé, mais performances variables ; ne pas déployer sur TRIGKEY.
- Correctif Crash Guard sur `src/arch/x86_64/idt/exceptions.rs` : un crash utilisateur WebContent a été récupéré sans `KERNEL PANIC` pendant la campagne observée, avec relance d'un WebContent par Ladybird.
- Assertion LibJS toujours ouverte : `LibJS/Runtime/Value.cpp:484`, dans `JS::Value::to_utf16_string`, sur le commit upstream `cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6`. Ne pas supprimer l'assertion.
- Refonte UI P19 V3 : tests C++ machine à états PASS, compilation Ladybird `2788/2788` PASS, nouveau `BouchaudBrowserHost` exporté dans `native-browser-m9`.
- QEMU Baseline de la nouvelle UI : Google s'affiche et fonctionne visuellement ; YouTube reste partiel (skeleton). Captures utilisateur : deux barres visibles, une dans le noyau et l'autre dans Ladybird.
- Log `baseline-20261012-202709` examiné hors dépôt : 13 événements `[LB:LOAD]`, dont des `start_duplicate`, sans `KERNEL PANIC`, `PROCESS_FAULT` ni `VERIFICATION FAILED` dans cette capture de campagne. La fin du fichier ne montre pas d'arrêt propre ; cela ne prouve pas une longue endurance.
- Les `start_duplicate` signalent des callbacks répétés ; ne **prouvent pas** à eux seuls une vraie boucle de navigation du document.
- CPU élevé et FPS relativement bas sous QEMU TCG ; ne pas extrapoler aux performances matérielles TRIGKEY.
- Aucun flash TRIGKEY ni validation physique P19.

## 4. Diagnostic des DEUX barres

- Barre supérieure dans la surface, chronomètre près du titre `Ladybird` : noyau Rust, `src/gui/widgets.rs::dessine_jauge` piloté par `src/gui/jauge.rs`.
- Barre fine sous l'URL et points par onglet : frontend C++, `tools/ladybird/chrome/BouchaudChrome.h`, états dans `BouchaudChargement.h`.
- La seconde barre du noyau n'est pas un indicateur réseau/DOM : elle mesure les trames graphiques et peut redémarrer sur de l'animation.
- Correctif envisagé : garder la jauge pendant le *démarrage initial* et sa télémétrie, puis cesser de dessiner la barre du noyau par-dessus les pages actives.
- Ne pas confondre avec le liseré actif sous un onglet ou dans la barre des tâches.

## 5. Prochaine séance : ordre recommandé

1. Faire le point sur le HEAD réellement poussé et vérifier `git status --short`.
2. Contrôler si ONE-LOADER est dans le diff ; sinon appliquer le correctif existant **sur une copie de travail propre**, prévalidation puis tests Rust.
3. Construire noyau + nouvelle image QEMU isolée, conserver `ladybird-browser.img` original (SHA connu `947d7170a217bb69553906d4c0f6d3d46e69c20b449619e5a06d8588e1497584` dans l'expérience précédente).
4. Vérifier visuellement une seule barre de navigation et la disparition de l'overlay noyau sur une page active.
5. Instrumenter `start/user_start/start_duplicate/finish/stop` par page/génération/URL réelle pour diagnostiquer YouTube sans supposer une boucle infinie.
6. Corriger ensuite LibJS `Value.cpp:484` avec instrumentation de tag/valeur, sans `VERIFY_NOT_REACHED()` neutralisé.
7. Retester P13/P19 : primitives, mémoire WebContent, robustesse, Compositor, endurance TCG puis KVM, avant le physique.

## 6. Contraintes non négociables

- Ne pas utiliser `git reset --hard`, `git clean -fd`, force-push, rebase destructif, ni travailler sur `main`.
- Les images binaires, caches vcpkg/WSL, `native-browser-m9`, `third_party/caches`, `target/` et les traces privées restent hors Git.
- Si une CI est rouge, conserver le rouge et corriger la cause ; pas de faux vert.
- La preuve STATIC/HOST n'est pas une preuve QEMU, encore moins une preuve TRIGKEY.
- Privilégier l'isolation des worktrees et des images pour les essais risqués.

## 7. Artefacts uniquement locaux (non poussés)

- Sauvegarde automatique de pré-push dans `target/handoff-backups/20261012-220009-b9ce2a` sur le poste d'origine.
- Rapports `target/p19-perf-ab/`, `target/p19-loading-ui-test/`, zip diagnostics et images QEMU : non versionnés.
- Fichiers non suivis laissés sur place (inventaire de pré-push) : `1`.
  - `third_party/caches/HSTSPreload/transport_security_state_static.json`

## 8. Expérience immédiate à réaliser avec Claude

Hypothèse : la ligne de chargement redondante vient de l'overlay du noyau et non du frontend P19.
Test : build du noyau avec et sans `dessine_jauge()` sur pages actives, même ramdisk et même scénario QEMU Baseline.
Preuve : captures avant/après, nombre de barres à l'écran, `[LB:LOAD]`, `KERNEL PANIC`, `PROCESS_FAULT`, CPU et FPS.
Échec : barre noyau encore présente, régression démarrage ou baisse de stabilité.
