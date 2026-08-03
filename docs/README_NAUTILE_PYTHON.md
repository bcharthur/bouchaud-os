# Nautile en Python

Le navigateur Nautile est desormais ecrit en Python et s'execute dans
Bouchaud OS par l'interpreteur embarque (RustPython compile en WASM, voir
`src/lang/python.rs`). Le noyau ne fournit plus que ce qu'il est seul a savoir
faire : le reseau, le framebuffer et les evenements d'entree.

```
bsh> nautile                      # page d'accueil
bsh> nautile example.com          # ouvre une URL
bsh> nautile "chat botte"         # recherche depuis la barre d'adresse
bsh> nautile example.com --dump   # rendu texte, meme sous le bureau
```

Sans framebuffer (shell texte), la commande ajoute `--dump` d'office : le seul
rendu possible est alors le texte. Sous le bureau graphique, elle passe
`--backend bouchaud` et le navigateur peint dans le framebuffer.

## Pourquoi des pseudo-fichiers

Le module WASM ne peut appeler le noyau que par les fonctions WASI qui lui sont
exposees, et ici ce sont les fonctions **fichiers** — `path_open`, `fd_read`,
`fd_write` — traduites vers le RAMFS par `src/wasm/mod.rs`.

Un module Python natif serait plus direct, mais `rustpython.wasm` est
precompile hors ligne (voir `tools/python-wasm/README.md`) : lui en ajouter un
imposerait de reconstruire tout l'interpreteur. Le pont emprunte donc le seul
canal deja ouvert.

`src/browser/pybridge.rs` cree trois nœuds dans le RAMFS et intercepte les
lectures et ecritures qui les visent :

| Chemin | Sens | Role |
| --- | --- | --- |
| `/dev/nautile/net` | ecriture puis lecture | requetes reseau |
| `/dev/nautile/gpu` | ecriture | commandes de dessin |
| `/dev/nautile/events` | lecture | evenements clavier et souris |

L'interception se fait dans `wasi_fd_write` et `wasi_fd_read` : si l'inode vise
est un peripherique, le noyau agit au lieu de toucher au contenu du fichier.

## Reseau

Python ecrit `GET <url>`, puis relit le fichier. Le noyau execute la requete
avec sa pile habituelle — DNS, TCP smoltcp, TLS 1.3 maison — et depose une
**reponse HTTP brute** que Python reanalyse avec son propre analyseur.

Le corps est deja dechunke et decompresse par `net::encoding` ; la reponse
n'annonce donc ni `Transfer-Encoding` ni `Content-Encoding`. Il n'y a ainsi
qu'un seul analyseur de reponse dans le projet Python, partage avec le
transport socket utilise hors de l'OS.

## Dessin

Une frame produit un bloc de commandes ecrit en une fois : un aller-retour par
primitive couterait bien plus cher que le dessin lui-meme.

```
rect <x> <y> <w> <h> <rgb>
round <x> <y> <w> <h> <rayon> <rgb>
text <x> <y> <taille> <fanions> <rgb> <texte jusqu'a la fin de ligne>
image <x> <y> <w> <h> <src>
clip <x> <y> <w> <h>
unclip
flush
```

Les couleurs sont des entiers `0xRRGGBB` ecrits en decimal. Les fanions de
`text` forment un champ de bits : `1` gras, `2` italique, `4` chasse fixe,
`8` souligne, `16` barre.

Nautile pose le texte sur sa **ligne de base** ; `draw_text_prop` attend le
haut du cadratin. Le pont convertit avec une ascendante de 80 % de la taille.

`flush` appelle `present()` : le noyau peint hors-ecran et ne bascule qu'a ce
moment, donc pas de scintillement.

## Evenements

Le noyau empile les evenements avec `pybridge::push_click`, `push_scroll`,
`push_key`, `push_resize` et `push_navigate`. Une lecture de
`/dev/nautile/events` depuis la position zero en consomme un ; les lectures
suivantes voient la fin de fichier. La file est bornee a 64 entrees pour qu'un
navigateur qui ne lit plus ne la fasse pas grossir sans fin.

## Installation du paquet

Les fichiers Python sont embarques dans le binaire par `include_str!`, comme
les polices et les certificats CA, et ecrits dans `/usr/lib/python/nautile` au
demarrage par `pybridge::install()`. La commande shell lance le paquet via
`python -c` plutot qu'en donnant le chemin de `__main__.py` : un fichier lance
directement n'est pas un paquet, et ses imports relatifs echoueraient.

Le source de reference vit dans le depot
[nautile-navigateur](https://github.com/bcharthur/nautile-navigateur) ; la copie
de `src/assets/python/nautile/` en est le vendoring, comme l'etait
`src/browser/` pour l'ancien moteur Rust.

## Etat

La commande `nautile` fonctionne, en mode texte comme en mode graphique.

L'**application de bureau** (`browser.bapp`, `src/gui/apps/`) utilise encore
l'ancien moteur Rust de `src/browser/`. Les deux coexistent volontairement : le
moteur Rust rend aujourd'hui des pages que le moteur Python ne rend pas encore
aussi bien, et l'inverse est vrai pour la cascade CSS. Basculer l'application de
bureau sur le pont est l'etape suivante ; `pybridge::push_*` en est l'API.

Le moteur Python **n'execute pas JavaScript** : les pages qui construisent leur
contenu au chargement s'affichent vides.
