# Atelier — handoff (reprise de session)

**Branche** : `claude/atelier-experience-20261103`, issue de `main@69e179a4`. Le design est décrit dans [ATELIER_DESIGN_SYSTEM.md](ATELIER_DESIGN_SYSTEM.md). Aucune PR n'est ouverte.

## Reprendre en 3 commandes

```bash
tools/ci/build_kernel.sh                                    # bootimage BIOS
tools/ci/run_bureau_capture.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin /tmp/cap
BUREAU_EXPORTS="export BO_ATELIER_OUVRE=1,4,9" tools/ci/run_bureau_capture.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin /tmp/cap4
```

La capture est dans `/tmp/cap/bureau.png`. Pour la voir, ouvrir le PNG. `kind` : 0 Terminal, 1 Fichiers, 3 Moniteur, 4 Calculatrice, 5 Rustpad, 7 Journal, 8 Services, 9 Notes, 10 Insights (le navigateur 6 est exclu).

## Vérifier avant de pousser

```bash
for t in tools/gui/test_*.rs; do n=$(basename $t .rs); rustc --edition 2021 --test -o /tmp/$n $t && /tmp/$n --test-threads=1 | grep 'test result'; done
tools/ci/run_architecture_guards.sh | tail -1     # attendu 209/209
python3 tools/ci/check_pipeline_policy.py         # CI_POLICY_OK
python3 tools/mesure-chantiers.py --ecris         # si un garde-fou le demande
```

## Où est quoi

| Besoin | Fichier |
|---|---|
| Valeurs (couleurs, rayons, durées) | `src/gui/atelier/jetons.rs` |
| Courbes et interpolations | `src/gui/atelier/mouvement.rs` |
| Géométrie du dock | `src/gui/atelier/dock.rs`, consommée par `window::start_btn(n)` / `taskbar_btn(i, n)` |
| Fond | `src/gui/atelier/fond.rs` (cache unique ; marqueur `BOUCHAUD_ATELIER_FOND_CALCULE`) |
| Verre, ombre, corps de fenêtre | `src/gui/atelier/matiere.rs` |
| Peintres branchés | `src/gui/widgets.rs` : `draw_wallpaper`, `draw_filigrane`, `draw_topbar`, `draw_taskbar` (dock), `draw_window`, `draw_window_controls` |
| Calques translucides | `src/gui/window_manager.rs` `plan_de_scene` : `BarreHaute` et `BarreTaches` sont `Calque::transparent` |

## Pièges déjà rencontrés (ne pas les redécouvrir)

1. **Calque opaque sous du verre** : `Calque::plein` fait sauter le fond sous lui. Tout ce qui est translucide doit être `transparent`.
2. **Flottant par pixel sous TCG** : +2,3 s à la première trame. Par pixel : entier seulement (`fond::mix`).
3. **Cache d'icônes** : un emplacement par taille posée (`icones::rang`). Une nouvelle taille exige un emplacement neuf, sinon les icônes du bureau prennent la taille du dock.
4. **Survol** : un déplacement de souris n'invalide que le curseur. Tout effet de survol doit invalider l'ancien et le nouvel élément (c'est ce que fait déjà `hovered_button_rect` pour les boutons de fenêtre).
5. **Garde-fous de la barre haute** : `verifie-lien-reseau.py` exige une mise en page droite-gauche (`let mut droite`, ≥ 2 `droite = droite.saturating_sub`), `dessine_reseau`, la barre oblique de l'état déconnecté et `frame_clock::snapshot()` dans `draw_topbar`. `tools/dev/verifie-v15.py` exige `FPS:{:3}`. Le FPS reste donc visible, discret.
6. **`verifie-chargement-unique.py`** exige `dessine_duree_chargement` dans `draw_window`.
7. **Socket moniteur QEMU** : 108 octets maximum. Le banc utilise `/tmp/bo-moniteur.*`.
8. **`socat` est absent du conteneur** : utiliser `tools/ci/qemu_moniteur.py SOCKET "commande"`.
9. **Applications natives** : elles peignent pour une surface sombre. Un corps de fenêtre clair rend le terminal illisible, d'où le corps sombre jusqu'à l'incrément 3.

## Prochain incrément (2 — mouvement), plan précis

1. `atelier::mouvement` : ajouter un `Registre` borné (8 interpolations au plus, sans allocation) avec `actif(maintenant) -> bool` et `degats(maintenant) -> Rects`.
2. `window_manager::boucle` : si `registre.actif`, demander la prochaine trame à `PERIODE_TRAME_MS`, sinon garder la veille actuelle. Ajouter un test hôte prouvant qu'un bureau immobile ne produit aucune trame.
3. Survol du dock :
   - à chaque mouvement de souris, calculer la cellule survolée (`atelier::dock`) ;
   - si elle change, invalider l'ancienne et la nouvelle cellule, puis lancer un fondu de 120 ms ;
   - mettre à jour `garde-fous` et `test_transitions` (aligner son modèle de boutons sur `atelier::dock`).
4. Ouverture et fermeture de fenêtre :
   - interpolation d'opacité sur 240 ms, en sortie cubique ;
   - garder la contrainte « jamais au-delà de `painted_bounds` » ;
   - pas d'échelle tant que la surface du client n'est pas réductible sans copie.
5. Capture en série : 3 `screendump` à 0, 60 et 240 ms après un clic injecté (`qemu_moniteur.py "mouse_button 1"`).

À ne pas toucher : P13, `kernel::update`, ordonnanceur, sécurité, `paint_window_shape_spans` (tests hôte) et la cadence du compositeur au repos.
