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
| M9 | HTTP reel via RequestServer | **en cours** |

### Ce que M8 prouve exactement

    Window Manager -> surface partagee -> /bo-navigateur -> webcontent-bootstrap
      -> WebContent (Ladybird) -> PageHost/PageClient -> DOM -> layout -> paint
      -> Skia CPU -> bitmap BGRA -> conversion XRGB -> surface -> FrameReady -> ecran

Pas Chromium. Pas Qt. Pas une WebView. Le vrai moteur d'upstream, compile pour
Bouchaud, affichant dans une fenetre de Bouchaud.

## 3. M9 — ou en est-on precisement

M9 est le jalon en cours. Il est plus avance que son voyant rouge ne le suggere,
et le detail compte parce qu'il dit ou chercher.

**Acquis, mesures :**

    M9_REQUESTSERVER_LAUNCHED / READY / CONNECTED   RequestServer natif tourne
    "GET /m9.html HTTP/1.1" 200                      la requete part de Bouchaud
    M9_RS_REQUEST_STARTED  id=0                      le tube de reponse traverse
    M9_RS_HEADERS  id=0 statut=200 nb=6              en-tetes recues
    M9_RS_REQUEST_FINISHED id=0 taille=672           corps complet recu

Toute la chaine reseau fonctionne, **y compris** le mecanisme le plus exigeant :
RequestServer fabrique une paire de sockets, en passe le bout lecteur a
WebContent par **SCM_RIGHTS**, et y ecrit le corps.

**Non acquis :** le document n'est jamais commite. WebContent se fige, et QEMU
le tue au bout de 240 s (`code 124`).

**Ce qui a ete elimine, definitivement :**

- ce n'est pas le reseau — la fixture repond 200 ;
- ce n'est pas le transfert du corps — 672 octets arrivent ;
- ce ne sont pas les en-tetes — statut et six en-tetes arrivent ;
- ce n'est pas une annulation — `page_did_cancel_loading` ne se declenche pas ;
- ce n'est pas `FIONBIO`, `setsockopt` ni `MSG_NOSIGNAL` — verifies dans le noyau ;
- ce n'est pas la comptabilite de `poll` sur une paire — `etat_pair()` signale
  bien la fermeture du pair.

**Piste en cours de mesure :** `WebContentClient.ipc` declare vingt-cinq
messages **synchrones** vers l'interface navigateur, et
`ConnectionBase::wait_for_specific_endpoint_message_impl` les attend **sans delai
d'expiration**. Une navigation `http://` en declenche que `load_html` — donc M8 —
ne declenche jamais : cookies, HSTS, stockage. Le peer M9 n'est pas une interface
complete. Le port court-circuite deja `decide_navigation_process` pour cette
raison exacte.

Une sonde nomme desormais chaque attente synchrone. Celui qui ne revient jamais
sera le dernier imprime.

Le chronometre appuie la piste :

    09:49:28  navigation commence
    09:49:38  la requete part enfin        (+10 s : une attente, pas un calcul)
    09:49:39  reponse complete
    ...       gel jusqu'au delai

## 4. Ce qui n'est pas fait, et qu'il ne faut pas croire fait

- **Le navigateur historique** (Python + QuickJS + Qt) reste le chemin par
  defaut. Ladybird n'est pas encore le moteur du produit.
- **Aucune isolation.** Le sandbox d'upstream est volontairement remplace par
  l'implementation non effective. C'est M14.
- **Pas de HTTPS.** Ni certificats, ni validation X.509.
- **Un seul onglet, un seul renderer.**
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
