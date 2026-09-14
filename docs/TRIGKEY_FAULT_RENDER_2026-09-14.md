# Correctif Trigkey : double faute et rendu professionnel

Date : 2026-09-14

## Diagnostic physique

La photo du Trigkey montre une double faute (vecteur 0x8) après le dernier
point de contrôle `reseau`. La pile affichée est la pile interrompue et peut
encore être la pile d'amorçage UEFI : l'ancien libellé `HORS TAS NOYAU`
n'établissait donc pas à lui seul une corruption.

La régression introduite dans le dernier lot était le dessin d'une étape de
progression GOP depuis chaque `point_de_controle`. Ces points s'exécutent
pendant une phase où les interruptions, la pile courante et le propriétaire du
framebuffer ne sont pas encore uniformes. Le point de contrôle redevient une
simple écriture diagnostique sans verrou. Le logo de démarrage reste dessiné
une seule fois avant la prise de contrôle par le pilote graphique.

## Rendu

L'interface normale utilise désormais DejaVu Sans via le rasteriseur TrueType
anticrénelé dans le terminal, le journal, le moniteur, les services, la
calculatrice, l'explorateur, Rustpad et l'écran d'arrêt.

L'écran de faute ne peut ni allouer, ni verrouiller le cache TTF. `build.rs`
rasterise donc DejaVu Sans à la compilation dans six atlas alpha immuables.
Le gestionnaire de double faute ne fait ensuite que mélanger ces pixels dans le
GOP. Les séquences de couleur ANSI sont filtrées avant affichage.

Les icônes Services, dossier et fichier sont décrites par des formes
géométriques et rendues avec sous-échantillonnage, ce qui évite les escaliers
visibles des anciens rectangles au pixel.

## Validation matérielle attendue

1. Construire Ladybird localement puis l'USB avec `build-ladybird-local.ps1 -BuildUsb`.
2. Vérifier que le logo apparaît puis que le bureau s'ouvre sans écran de faute.
3. Ouvrir Explorateur, Rustpad, Terminal, Moniteur, Services et Calculatrice.
4. Vérifier les contours de glyphes et d'icônes aux tailles normale et petite.
5. En cas de faute, photographier l'écran complet : les dernières lignes ne
   doivent plus contenir de séquences `[90m` ou caractères `?` parasites.

## Deuxieme demarrage physique

Le second essai atteint `bureau` (11 points franchis) puis tombe en double
faute avec un RSP termine par `0x850`. Ce jalon se trouve juste avant
`task::run_noyau`, qui fabrique le premier cadre de pile du bureau.

L'ancien cadre consommait exactement huit mots entre `ctx.rsp` et l'entree du
trampoline. Avec un sommet aligne a 16 octets, le trampoline commencait donc
avec `RSP % 16 == 0`, en contradiction avec l'ABI SysV x86-64 qui exige 8 a
l'entree d'une fonction. Une case de retour fictive est maintenant reservee :
apres les sept restaurations et le `ret`, le trampoline commence a
`RSP % 16 == 8`.

Des jalons sans journalisation couvrent desormais la creation de pile,
l'installation de la tache, le switch, le trampoline, l'entree du bureau et la
prise du framebuffer. Le panneau de faute montre aussi la tache, PID/TID, les
bornes de pile, l'alignement, la premiere exception eventuelle et une piste
principale. L'analyseur ANSI distingue maintenant correctement `ESC [` des
octets finaux CSI.

Le B du prechargeur UEFI et celui du noyau ne sont plus six rectangles. Ils
utilisent la meme forme vectorielle, echantillonnee en 4x4 puis melangee au
fond avant affichage.
