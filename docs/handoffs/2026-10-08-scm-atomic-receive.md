# Reception SCM_RIGHTS atomique — reprise du 8 octobre 2026

P13 et P2/P9 restent ouverts. P10 inchange. Aucun essai PHYSICAL.

## Etat de reprise preserve

Le fetch initial confirme `952e95d6261d4e24bf2a252f74a7bf11f60eb8fb`.
Le worktree `bouchaud-os-control` est detache sur ce commit, avec le correctif
`src/compat/linux/net.rs` deja indexe et des notes non suivies. Il est conserve.
Le worktree `bouchaud-os-verified` conserve les trois diagnostics memoire
indexes decrits dans le handoff precedent. Aucun n'est publie dans ce correctif.

Checkout actif : `/workspace/scratch/bf8518044641/bouchaud-os`, branche unique
`claude/ladybird-observability-performance`. Aucun reset, clean, rebase,
force-push ni travail sur main. Les publications utilisent un parent exact,
un arbre compare a l'index local et `force=false` avec SHA distant attendu.

## Campagne instrumentee terminee AVANT le correctif

Run [37807102697](https://github.com/bcharthur/bouchaud-os/actions/runs/37807102697),
HEAD `63eef99c1cd29fe5318d689a7c699def6ffa46f9`.

| Job | Resultat | Observation |
| --- | --- | --- |
| build, primitives, WPT, cache/SQL | success | Noyau avant correction |
| crash services, ordre worker, piege Compositor | success | Instrumentation active |
| browser-host smoke, sites reels, performance | success | Aucun budget change |
| robustesse | failure | Fin du cycle worker : sorties comptees 8/9 |
| TCG 10 min | failure | Vraie sortie du Compositor PID 18, remplace par 153 |
| KVM A/B | failure | A passe ; ligne FIN de B entrelacee avec SCHED-TACHE |
| memoire | failure | Croissance du WebContent principal |
| KVM 20 min | success | 227 cycles, 75/75 swaps, 1201 s |
| convergence | failure | Les quatre rouges restent rouges |

KVM 20 min : `created=1 removed=0 live=1 pid=18`, 1171 echantillons ;
stabilite et performance passent. Les deux seules traces CONTROL sont OPEN.
Artefact `11566431022`, SHA256
`60edd0cb6765af28eb088e68581a3f3e3b1b955c245198c88fe9dd6158802249`.

KVM A/B : les deux bras conservent un seul Compositor ; aucune fermeture
CONTROL dans leurs journaux. Le bras A passe avec 57 cycles et 19/19 swaps.
La ligne `HOST_ENDURANCE_FIN` du bras B, ligne 23129, est entrelacee octet par
octet avec un diagnostic scheduler. Le verdict reste failure : ne pas
reconstituer ses valeurs pour le rendre vert. Artefact `11564564989`, SHA256
`bcebf045916ab2320a25a9de304840fab6e70ffdffe13b606e76f0be5e9c2ff9`.

## Attribution de la fermeture TCG

Artefact `11565656793`, `serie-endurance.log`, SHA256 du ZIP
`032dae01e1b6229b5b819a005054b71af34781ea64226f5b00edf5ee9d3b76dc`.

| Ligne | Evenement |
| --- | --- |
| 39081 | UI PID 15, fd 21 : READ_EOF_PUBLISHED, cause CONTROL_CHANNEL_EOF |
| 39082-39089 | UI : shutdown, fermeture locale puis UI_CONTROL_DIE, meme premiere cause |
| 39110 | Compositor PID 18, fd 26 : READ_EOF_PUBLISHED, cause CONTROL_CHANNEL_EOF |
| 39126-39130 | Compositor : shutdown, COMPOSITOR_DIE_REASON puis COMPOSITOR_EXIT |
| 39131 | Sortie noyau PID 18, code 0, t=568512 ms |
| 39151 | Nouveau Compositor PID 153 : CONTROL_CHANNEL_OPEN |

La premiere cause retenue est EOF, pas une erreur de protocole, une assertion,
un POLLHUP initial ni une destruction locale volontaire. La fermeture cote
UI precede les traces EOF et la sortie du Compositor. Ce constat attribue la
sortie normale a la fermeture du transport de controle.

Le lien avec SCM_RIGHTS repose sur la reproduction syscall independante et
le chemin de code : le journal Ladybird ne contient pas les valeurs brutes
octets/droits de chaque recvmsg. Ne pas le presenter comme une capture de
ces valeurs sur le fd 21. L'incident KVM historique non instrumente reste
non attribuable retrospectivement a son premier appel fautif.

## Defaut ABI et correction minimale

Sur `952e95d`, run [37809241418](https://github.com/bcharthur/bouchaud-os/actions/runs/37809241418),
job `113421497910` :

```text
SCM_RECEIVE_RACE_FAIL round=262 bytes=1 rights=0 errno=0 flags=0 peer_open=1 END
SCM_RECEIVE_AFTER_SPLIT bytes=0 rights=1 errno=0 peer_open=1 END
```

Un seul envoi est autorise a la fois, aucun pair ne ferme et aucun nouvel
envoi ne precede le second recvmsg. Le temoin Linux passe 10000/10000.

`sendmsg` publiait deja octets et droits sous le verrou du canal. `recvmsg`
retirait les droits, relachait le verrou puis lisait les octets : une arrivee
entre les deux donnait un octet sans son droit. L'appel suivant installait
le droit et convertissait EAGAIN en zero. Ladybird traite ce zero comme EOF
avant d'examiner SCM_RIGHTS, puis ferme le transport.

Commit produit `c78be28b22c2185f0c053c0845728197b071bbd3` : un seul fichier,
40 insertions / 18 suppressions. Les deux retraits partagent le verrou ;
installation des fd et copie utilisateur restent dehors. MSG_DONTWAIT,
MSG_CMSG_CLOEXEC, capacite du tampon de controle, vrais EOF et bornes
d'attente restent conserves. Les files separant droits et octets ne sont pas
redessinees : ce commit corrige la race reproduite, pas toute l'ABI sockets.

## Validation du candidat — en cours

Premiere sonde sur `c78be28`, run
[37813731798](https://github.com/bcharthur/bouchaud-os/actions/runs/37813731798),
job `113436878202` : compilation noyau reussie, temoin Linux 10000/10000.
Le journal invite atteint `SCM_RECEIVE_RACE_OK rounds=10000 expected=10000`,
mais un diagnostic noyau interrompt la ligne avant END. Le parseur refuse
correctement `missing complete success record`. **Ce run reste failure.**

Commit sonde `1a47387a7110398dda5f221edcf3a94c6978b795` : trois copies du
meme verdict final, chacune en un write. Le workload reste execute une fois.
L'assertion exige toujours une ligne complete 10000/10000 et refuse tout
marqueur FAIL. Aucun timeout ni assertion n'est relache.

Nouvelle sonde : run [37814193790](https://github.com/bcharthur/bouchaud-os/actions/runs/37814193790),
job `113438467336`, **success**. Le noyau compile ; les trois lignes finales
invites sont completes : `SCM_RECEIVE_RACE_OK rounds=10000 expected=10000 END`.
Le parseur produit `SCM_RECEIVE_KVM_OK`. Aucun marqueur FAIL, aucune faute
processus ni panique noyau. Il s'agit d'un workload de 10000 echanges, pas
de trois repetitions du workload. Le temoin Linux passe egalement.
Artefact `11566441925`, SHA256
`58b3e3761640f47912bd7c54c83dbd9e1b77c156e52fb24268afe22c984bda05`.

CI Fast `37814193752`, Reliability V3 `37814193896` et Integration
`37814193720` sont success sur `1a47387`. Leurs executions sur `c78be28`
ont ete annulees automatiquement par le commit de sonde suivant ; elles
ne sont pas presentees comme des validations de ce HEAD.

Replay Ladybird : run `37813732807`,
HEAD technique `c78be28`. Les deux commits ont exactement le meme code noyau
et les memes preparateurs Ladybird ; seul l'emetteur de la sonde differe.
Les resultats doivent etre completes apres lecture des jobs et artefacts.
