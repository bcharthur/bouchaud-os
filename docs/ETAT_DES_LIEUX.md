# Bouchaud OS — etat des lieux

*Arrete au 18 aout 2026. Ce document ne dit que ce qui a ete **constate**, avec
l'endroit ou la preuve se relit. Tout le reste est dans `VISION.md`, qui est un
plan et s'annonce comme tel.*

## La regle de ce document

Une ligne verte ici veut dire : un temoin s'est execute en **ring 3, sous QEMU**,
a imprime son resultat sur la sortie serie, et un verdict de CI a exige cette
sortie ligne par ligne. Pas « ca compile ». Pas « le code existe ».

C'est une regle couteuse — elle a fait echouer plusieurs fois des jalons qu'on
croyait acquis — et c'est exactement pour cela qu'elle est tenue.

## 1. Le noyau

| Brique | Etat | Ou se relit la preuve |
|---|---|---|
| Noyau Rust `no_std`, x86-64, ring 3 | acquis | `ci`, `system-health` |
| ABI Linux : 153 appels systeme routes | acquis | `docs/linux-compat/SYSCALL_MATRIX.md` |
| `socketpair`, `sendmsg`/`recvmsg`, **SCM_RIGHTS** | acquis | temoin M5 + chemin M9 |
| `poll` / `select` avec reveil inter-fils | acquis | `docs/ladybird/M5_CODEC_RING3.md` |
| `mmap`, `MAP_FIXED`, `PROT_NONE`, pagination a la demande | acquis | `src/kernel/vmm.rs` |
| `clone3`, fils POSIX | acquis | temoin M5 |
| **Memory Fabric / VMA** : chargement paresseux des gros fichiers | acquis | `docs/architecture/MEMORY_FABRIC.md` |
| **Resource Core** : CPU, memoire, DMA, GPU sous une meme comptabilite | acquis | `docs/architecture/RESOURCE_CORE.md` |
| Gestionnaire de fenetres, seul proprietaire du tampon d'ecran | acquis | `src/gui/` |
| Pile TCP/IP, e1000 | acquis | `M9_FIXTURE_HTTP_OK` |

Le chargement paresseux n'est pas theorique. Au demarrage du disque Ladybird :

    tar: hdb deplie -> 41 fichiers, 17 repertoires, 332402 Kio
         (2 fichiers paresseux, 328509 Kio non copies)

Autrement dit : 320 Mio de binaires Ladybird sont **montes sans etre copies**.
Sans cela, le plafond d'archive de 192 Mio interdisait purement et simplement
d'embarquer WebContent.

## 2. Le portage Ladybird

SHA epingle : `cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6`. Jamais suivi par une
branche mouvante.

| Jalon | Contenu | Etat |
|---|---|---|
| M0 | Infrastructure, licences, CI de synchro | vert |
| M1 | AK | vert |
| M2 | LibCore **complet** (42 sources, pas un sous-ensemble) | vert |
| M3 | LibGC | vert |
| M4 | LibJS — l'interpreteur `js` d'upstream execute du JavaScript en ring 3 | vert |
| M5 | LibIPC, endpoints **generes par le generateur d'upstream**, 6 types | vert |
| M6 | LibGfx + Skia CPU — pixels BGRA en ring 3 | vert |
| M7 | WebContent demarre comme processus separe, poignee de main IPC | vert |
| M8 | HTML local rendu dans une vraie fenetre Bouchaud | vert |
| M9 | HTTP reel via RequestServer — page distante affichee | **vert** |
| M11 | Chrome utilisable : barre d'adresse, historique, liens, defilement | **construit, a eprouver sous QEMU** |
| M12a | HTTPS avec verification de chaine, fixture TLS locale | **vert** |
| M12b | Un site public joignable par son **nom** | corrige : demultiplexage UDP par socket |

### Ce que M8 prouve exactement

    Window Manager -> surface partagee -> /bo-navigateur -> webcontent-bootstrap
      -> WebContent (Ladybird) -> PageHost/PageClient -> DOM -> layout -> paint
      -> Skia CPU -> bitmap BGRA -> conversion XRGB -> surface -> FrameReady -> ecran

Pas Chromium. Pas Qt. Pas une WebView. Le vrai moteur d'upstream, compile pour
Bouchaud, affichant dans une fenetre de Bouchaud.

## 3. M9 — clos, et ce qu'il a coute

M9 est vert. La page distante est chargee par RequestServer, rendue par LibWeb
et Skia, et affichee dans une fenetre Bouchaud.

Le jalon a demande sept passes de mesure, et **trois hypotheses annoncees puis
refutees** : l'isolation de sites (deja neutralisee dans le port), la fin de
fichier sur une paire de sockets (la comptabilite du noyau etait juste ; le
defaut reel etait ailleurs et a ete corrige separement), et les en-tetes
manquantes (elles arrivaient : statut 200, six en-tetes).

Ce qui a fini par tomber, ce sont des defauts que seule une navigation reelle
pouvait faire sortir — la propagation de la fin de fichier apres le dernier
ecrivain d'une paire, la coordination de l'historique inter-documents, la
livraison du corps mise en pause et jamais relancee.

La lecon vaut d'etre gardee : chaque refutation a ferme une zone pour de bon.
C'est plus lent qu'une intuition juste, et infiniment plus rapide qu'une
intuition fausse qu'on n'a pas verifiee.

## 3 bis. M11 — le chrome, et ce qui reste a prouver

Le jalon M11 (`ladybird/M11_NAVIGATEUR.md`) ajoute a WebContent la barre
d'adresse, les boutons d'historique, le routage des entrees du gestionnaire de
fenetres vers le document, et un repeint cadence au lieu d'une capture unique.

**Ce qui est verifie hors QEMU** : l'en-tete du chrome compile en C++23 contre
l'arbre Ladybird epingle ; les quatre scripts de preparation s'appliquent au SHA
epingle et sont idempotents ; les trois implementations du protocole GUI —
noyau, hote Qt, chrome — s'accordent (`tools/verifie-protocole-gui.py`).

**Ce qui ne l'est pas encore** : le comportement reel sous l'ordonnanceur de
Bouchaud. La latence d'un clic, la fluidite du defilement et la tenue du canal
GUI sous charge ne se mesurent qu'en demarrant l'OS. Cette ligne restera ici
jusqu'a ce qu'une execution QEMU l'efface.

## 3 ter. M12 — HTTPS et resolution de nom

Le job M12 est passe le 18 aout : chaine reellement validee, nom d'hote verifie,
document distant affiche dans une fenetre Bouchaud. C'est la premiere fois — la
branche qui portait ce travail ne compilait pas, un `\0` interprete par Python
ayant depose un octet NUL dans le C++ genere.

La cause a ete confirmee sous LibDNS : `poll()` pompait 20 000 fois un anneau RX
vide, puis le premier socket UDP examine pouvait retirer et jeter une reponse
destinee au port d'un autre socket A/AAAA. La pile demultiplexe maintenant les
datagrammes avant de les mettre en file et dort sur une absence de trafic. Voir
`ladybird/M12_HTTPS.md`, section « Cause racine M12b ».

## 4. Ce qui n'est pas fait, et qu'il ne faut pas croire fait

- **Aucune isolation.** Le sandbox d'upstream est volontairement remplace par
  l'implementation non effective. C'est M14.
- **Un seul onglet, un seul renderer.**
- **Pas de redimensionnement de la fenetre du navigateur natif.** La surface est
  allouee une fois ; `Configure` est journalise, pas suivi.
- **Pas de modificateurs clavier.** Le pilote du bureau n'expose pas encore
  Ctrl/Alt separement : ni Ctrl+L, ni Ctrl+R.
- **La dette ICU** reste entiere : environ 40 Mio de donnees statiques par
  binaire. Le chargement paresseux l'a rendue supportable, il ne l'a pas
  supprimee.
- **Aucune accelefation materielle.** Skia tourne en CPU, et Bouchaud n'expose
  aucun GPU.

## 5. Les trois defauts que ce chantier a revele dans Bouchaud

Ils valent d'etre notes parce qu'aucun n'etait specifique a Ladybird — chacun
etait un defaut generique que seul un vrai programme pouvait faire sortir.

1. **`sys_poll` se rendormait sans regarder.** Apres une commutation reelle,
   la boucle tombait dans `hlt` sans reevaluer les descripteurs : le reveil d'un
   fil par un tube etait perdu jusqu'a la prochaine interruption materielle.
   Corrige pour `poll` **et** `select`.
2. **Le plafond d'archive de 192 Mio** interdisait d'embarquer un binaire
   Ladybird. Resolu par le chargement paresseux du Memory Fabric, pas par un
   contournement.
3. **La CI jetait son travail a chaque echec.** `actions/cache` n'enregistre
   qu'a la fin d'un travail reussi ; trente minutes de vcpkg partaient a la
   poubelle a chaque tentative, ce qui rendait l'iteration impossible.

## 6. Ce que la discipline a coute, et rapporte

Trois fois pendant le chantier M9, une cause a ete annoncee puis refutee par la
mesure suivante : l'isolation de sites (deja neutralisee), la fin de fichier sur
une paire de sockets (deja geree), les en-tetes manquantes (elles arrivent).

Chaque refutation a ferme une zone pour de bon. C'est plus lent qu'une intuition
juste, et infiniment plus rapide qu'une intuition fausse qu'on n'a pas verifiee.
