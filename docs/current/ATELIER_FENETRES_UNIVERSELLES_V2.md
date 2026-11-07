# Atelier — chrome universel des fenêtres (v2)

Le patch est applique dans `widgets::draw_window`, **le peintre commun** de
Ladybird, DOOM, Fichiers, Notes, Terminal, Moniteur et toutes les autres
fenetres. Le theme ne reside pas dans le moteur web ou le jeu.

- Barre de titre uniforme avec verre clair, liseret lumineux borne par le rayon;
- Focus indique par un trait indigo discret, inactif par couleur et encre calmes;
- Trois capsules arrondies fixes et survol explicite (fermeture rouge uniquement
  quand pointee), sans modification des hitboxes ni des dimensions utiles;
- Ombres Atelier deja existantes et forme AA conservees;
- Surfaces ring 3 recopiees sans filtre, sans surcharge CSS, sans changement de
  layout ni de cadence du compositeur.

`src/gui/atelier/fenetre_style.rs` est un contrat **pur**, teste par la suite
hote `tools/gui/test_atelier.rs`. Le dessin suit exactement les jetons et les
zones de dommages preexistantes.

## Preuve authentique QEMU

```bash
tools/ci/build_kernel.sh
bash tools/ci/run_atelier_windows_capture.sh \
  target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin \
  target/atelier-windows-proof
```

Sorties : `atelier-fichiers.png`, `atelier-notes.png`,
`atelier-ladybird.png`, `atelier-doom.png`, et `serie.log`. Ce script
fait de **vraies captures de l'ecran QEMU** via `screendump` et verifie
l'absence de panique. Si le navigateur ou le jeu ne sont pas dans l'image,
leur capture prouve seulement le comportement du bureau au clic, pas le
fonctionnement du binaire applicatif. Il ne faut jamais qualifier l'application
Ladybird/DOOM si elle n'a pas ete chargee et visible.

Aucun seuil de CI et aucune protection A/B, SMP, Échap ou IPC ne sont modifies.
