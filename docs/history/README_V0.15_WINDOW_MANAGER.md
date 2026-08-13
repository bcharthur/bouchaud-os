# Bouchaud OS V0.15 — Window Manager + apps natives + Bouchaud Browser

Premier vrai pas vers un OS **Windows-like** : le bureau passe d'un dessin fixe
a un **gestionnaire de fenetres** avec plusieurs fenetres simultanees, focus,
z-order, deplacement, fermeture, **menu Demarrer**, barre des taches, et des
**applications natives**.

> Objectif : Windows-**like** (l'aspect et le fonctionnement), avec un systeme
> d'applications maison. Pas Windows-**compatible** (lancer de vrais .exe) : cela
> demanderait un loader PE, l'API Win32, des processus user-mode, etc. — un autre
> chantier, tres lointain.

## Ce qui est implemente

- **Window manager** (`gui/mod.rs`) : boucle d'evenements unique, entree
  souris/clavier non bloquante (`keyboard::try_key`), fenetres :
  - deplacables (glisser la barre de titre),
  - **fermables** (bouton rouge `x`),
  - **focus / z-order** (cliquer une fenetre la passe au premier plan),
  - barre de titre coloree selon le focus.
- **Menu Demarrer** (bouton *Start*) : lance Terminal, Fichiers, Navigateur,
  Moniteur, ou quitte le bureau.
- **Barre des taches** : bouton Start + une tuile par fenetre ouverte (clic =
  mise au premier plan).
- **Applications natives** :
  - **Terminal** : REPL complet (reutilise le shell : pipes, `;`/`&&`/`||`,
    redirections, `$VAR`...), scrollback. `exit` ferme la fenetre.
  - **Fichiers** : navigateur a la souris (dossiers, `..`, apercu de fichier),
    respecte les droits Unix.
  - **Bouchaud Browser** : navigateur natif avec barre d'adresse. Pages :
    `about:bouchaud`, `about:system`, `file:/<chemin>` (lecture RAMFS). Les URL
    `http(s)://` affichent un message (reseau a venir).
  - **Moniteur** : infos systeme en direct.
- **Modele d'application** : un dossier `/apps` contient des manifestes `.bapp`
  (`name=`, `exec=`, `type=`, `permission=`) — visibles dans l'app Fichiers.

## Tester

```text
desktop
# Menu Demarrer (bouton Start en bas a gauche)
#  -> Terminal : ls ; cat /etc/passwd | grep root ; sysinfo ; exit
#  -> Fichiers : clique apps/ puis browser.bapp (apercu)
#  -> Navigateur : barre d'adresse, tape  about:system  Entree
#                  puis  file:/readme.txt  Entree
#  -> Moniteur : heure/uptime en direct
# Plusieurs fenetres a la fois : deplace-les, clique pour le focus,
# ferme avec le x rouge. Start -> Quitter pour revenir au shell.
```

## Limites connues / prochaines etapes

- Resolution VGA 13h (320x200, 256 couleurs). La haute resolution
  (1280x720+) demandera une **migration framebuffer/bootloader 0.11**.
- Les apps tournent encore dans le noyau (pas de vrais processus user-mode).
- Pas de redimensionnement de fenetre ni de drag&drop (a venir).
- Le navigateur ne fait pas (encore) de reseau : `http://` arrivera apres le
  driver **e1000** + TCP/HTTP.

## Roadmap Windows-like (rappel)

V0.15 Window manager + apps natives + Browser (ici)
V0.16 Drag&drop, redimensionnement, themes
V0.17 Framebuffer haute resolution (bootloader 0.11)
V0.18 Runtime .bapp + lanceur generique
V0.19 Disque persistant BFS
V0.20 Reseau e1000 -> ping/DNS/HTTP -> navigateur en ligne
V0.21 Processus user-mode + syscalls
...    Compat .exe (PE loader) : tres lointain, voire jamais en direct
