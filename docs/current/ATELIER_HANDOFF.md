# Atelier — handoff (reprise de session)

**Branches** :
- `claude/atelier-experience-20261103`, issue de `main@69e179a4`, porte la PR #242 (incréments 1 et 2) ;
- `claude/atelier-increment-a-20261104` en est issue (incréments A et suivants).

Le design est décrit dans [ATELIER_DESIGN_SYSTEM.md](ATELIER_DESIGN_SYSTEM.md).

## Reprendre en 3 commandes

```bash
tools/ci/build_kernel.sh                                    # bootimage BIOS
tools/ci/run_bureau_capture.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin /tmp/cap
BUREAU_EXPORTS="export BO_ATELIER_OUVRE=1,4,9" tools/ci/run_bureau_capture.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin /tmp/cap4
# interaction réelle : ouvrir le centre de contrôle puis capturer
BUREAU_GESTES=$'coin\nmouse_move -23 15\nmouse_button 1\nmouse_button 0\nattente 2\ncapture centre' \
  tools/ci/run_bureau_capture.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin /tmp/capc
```

La capture est dans `/tmp/cap/bureau.png`. Pour la voir, ouvrir le PNG. `kind` : 0 Terminal, 1 Fichiers, 3 Moniteur, 4 Calculatrice, 5 Rustpad, 7 Journal, 8 Services, 9 Notes, 10 Insights (le navigateur 6 est exclu).

## Vérifier avant de pousser

```bash
for t in tools/gui/test_*.rs; do n=$(basename $t .rs); rustc --edition 2021 --test -o /tmp/$n $t && /tmp/$n --test-threads=1 | grep 'test result'; done
tools/ci/run_architecture_guards.sh | tail -1     # attendu 209/209
python3 tools/ci/check_pipeline_policy.py         # CI_POLICY_OK
python3 tools/mesure-chantiers.py --ecris         # si un garde-fou le demande
```

## Assets (incrément 2)

```bash
python3 -m venv /tmp/va && /tmp/va/bin/pip install resvg-py pillow      # développement seulement
/tmp/va/bin/python tools/assets/fabrique-atelier.py                      # refait PNG + MANIFEST
python3 tools/verifie-assets-atelier.py                                  # ce que la CI vérifie
```

Pour ajouter une icône :
1. copier son SVG depuis `lucide-static` (npm) dans `assets/atelier/sources/lucide/` ;
2. ajouter une ligne `APPS` ou `SYMBOLES` dans la fabrique, puis relancer celle-ci ;
3. l'embarquer : `tuile!` dans `icones.rs`, ou une constante dans `atelier/assets.rs`.

Le fond se change dans la liste `FONDS` de la fabrique. Ajouter une variante 2560×1600 donnerait un fond net en UEFI, au prix d'environ +3 Mo d'image.

## Où est quoi

| Besoin | Fichier |
|---|---|
| Valeurs (couleurs, rayons, durées) | `src/gui/atelier/jetons.rs` |
| Courbes et interpolations | `src/gui/atelier/mouvement.rs` |
| Géométrie du dock | `src/gui/atelier/dock.rs`, consommée par `window::start_btn(n)` / `taskbar_btn(i, n)` |
| Fond | `src/gui/atelier/fond.rs` (photographie, décodée une fois ; marqueur `BOUCHAUD_ATELIER_FOND`) ; recadrage pur dans `atelier/echelle.rs` |
| Assets embarqués | `src/gui/atelier/assets.rs` (logo, symboles, fond) ; tuiles dans `src/gui/icones.rs` |
| Verre, ombre, corps de fenêtre | `src/gui/atelier/matiere.rs` |
| Peintres branchés | `src/gui/widgets.rs` : `draw_wallpaper`, `draw_filigrane`, `draw_topbar`, `draw_taskbar` (dock), `draw_window`, `draw_window_controls` |
| Calques translucides | `src/gui/window_manager.rs` `plan_de_scene` : `BarreHaute` et `BarreTaches` sont `Calque::transparent` |

## Pièges déjà rencontrés (ne pas les redécouvrir)

1. **Calque opaque sous du verre** : `Calque::plein` fait sauter le fond sous lui. Tout ce qui est translucide doit être `transparent`.
2. **Flottant par pixel sous TCG** : +2,3 s à la première trame. Par pixel : entier seulement (`fond::mix`).
3. **Cache d'icônes** : un emplacement par taille posée (`icones::rang`). Une nouvelle taille exige un emplacement neuf, sinon les icônes du bureau prennent la taille du dock.
4. **Survol** : un déplacement de souris n'invalide que le curseur. Tout effet de survol doit invalider l'ancien et le nouvel élément (c'est ce que fait déjà `hovered_button_rect` pour les boutons de fenêtre).
5. **Garde-fous de la barre haute** : `verifie-lien-reseau.py` exige :
   - une mise en page de droite à gauche (`let mut droite`, au moins 2 `droite = droite.saturating_sub`) ;
   - `dessine_reseau` qui appelle `glyphe_reseau` ;
   - trois glyphes distincts (connecté, débranché, alerte) ;
   - aucun dessin à la main ;
   - `frame_clock::snapshot()` dans `draw_topbar`, sous `reglages::diagnostic()`.

   `tools/dev/verifie-v15.py` exige `FPS:{:3}`.
6. **`verifie-chargement-unique.py`** exige `dessine_duree_chargement` dans `draw_window`.
7. **Socket moniteur QEMU** : 108 octets maximum. Le banc utilise `/tmp/bo-moniteur.*`.
8. **`socat` est absent du conteneur** : utiliser `tools/ci/qemu_moniteur.py SOCKET "commande"`.
9. **Applications natives** : elles peignent pour une surface sombre. Un corps de fenêtre clair rend le terminal illisible, d'où le corps sombre jusqu'à l'incrément 3.
10. **Souris QEMU** : un grand `mouse_move` laisse un reliquat qui absorbe les gestes suivants. Utiliser l'instruction `coin` de `BUREAU_GESTES` (pas de 120), puis des déplacements relatifs. La souris n'a pas d'accélération et est bornée à l'écran.
12. **Animations** : rien ne s'anime avant la première trame du bureau, et une animation est datée à sa première trame composée. Pour capturer une animation en cours, utiliser `capture` dans `BUREAU_GESTES`, qui fige la VM. Le timing de l'hôte varie : prévoir des captures serrées (`attente 0.02`).
11. **Centre de contrôle** : toute nouvelle commande doit être réelle, ou affichée comme non prise en charge. La géométrie va dans `atelier::centre`, sinon le clic et le survol divergent du dessin.

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
