# Contrôle distant et télémétrie — Bouchaud OS

Ce document décrit le plan de contrôle de la machine de référence TRIGKEY. Il
complète `docs/BOUCHAUD_LAB_REMOTE.md`, qui documente le protocole BRDP et le
canal de survie UDP.

## Deux canaux, deux rôles

| canal | transport | rôle | authentification | dépend de la RX |
|---|---|---|---|---|
| BRDP | TCP 2222 | lire l'état et déclencher des actions bornées | HMAC-SHA256 | oui |
| télémétrie | UDP 2223 broadcast | événements et heartbeat de survie | non | non pour émettre |

La télémétrie n'est pas un canal de commande. Elle est volontairement
unidirectionnelle et non authentifiée : on l'écoute, on ne lui envoie rien.
BRDP est le seul plan de contrôle et exige `BOUCHAUD_DEBUG_TOKEN`.

Une image LAB contient ce secret de build. Elle ne doit pas être distribuée.
Il n'existe volontairement **aucun shell distant** et aucune commande `exec`
arbitraire.

## Préparer le poste Windows

```powershell
cd C:\Users\Arthur\RustroverProjects\bouchaud-os-main
$env:BOUCHAUD_DEBUG_TOKEN = "<secret de l'image LAB>"
```

L'adresse LAB de la machine de référence actuellement utilisée est :

```text
169.254.178.21
```

Elle reste découvrable sans connaître l'adresse à l'avance :

```powershell
python .\tools\remote\bouchaud-lab.py discover --timeout 10
```

`discover` est passif. Il écoute UDP 2223 et n'émet aucun paquet.

## Télémétrie : toujours la lancer avant un essai physique

Dans une console dédiée :

```powershell
python .\tools\remote\bouchaud-lab.py telemetry --watch
```

Exemples de lignes utiles :

```text
telemetry=1 heartbeat=oui t_ms=...
remote BRDP_ECOUTE port=2222
remote BRDP_AUTH ok=oui raison=0
remote BRDP_CONTROLE commande=100 cible=0 acceptee=oui
rtl8168 RING_SECOND_LAP_TIMEOUT ...
audit AUDIT_SAIN tours=...
```

Le heartbeat prouve que le noyau émet encore. Il ne prouve pas que TCP 2222
est déjà prêt. Après un reboot physique, le premier heartbeat et `BRDP_ECOUTE`
peuvent précéder une fenêtre transitoire de readiness réseau. Le critère final
est une vraie commande BRDP authentifiée (`status`).

## Lire l'état

```powershell
python .\tools\remote\bouchaud-lab.py --json status `
    --host 169.254.178.21 `
    --timeout 10

python .\tools\remote\bouchaud-control.py doctor
```

`doctor` regroupe `status`, `services`, `processes`, `memory`, `net`, `rtl8168`
et l'état de la preuve Internet. Une preuve Internet en `stage=idle` n'est pas
une panne réseau : elle signifie seulement qu'aucune génération de cette preuve
n'a été lancée.

## Capturer le journal série RAM par BRDP

```powershell
python .\tools\remote\bouchaud-lab.py serial-capture `
    --host 169.254.178.21 `
    --bytes 131072 `
    --out .\target\serial-live.log
```

La capture n'est possible que tant que BRDP répond. Pour une panne RX, la
console `telemetry --watch` reste donc la trace prioritaire.

## Contrôler Ladybird

```powershell
python .\tools\remote\bouchaud-control.py browser stop
python .\tools\remote\bouchaud-control.py browser start
python .\tools\remote\bouchaud-control.py browser restart
```

Depuis P0 Remote Control V1.2, `browser restart` est **deux phases** :

1. un tour du Window Manager arrête et détache l'ancien arbre Ladybird ;
2. le Window Manager rend la main au scheduler et programme `DEMARRER` ;
3. le tour suivant lance le nouveau Ladybird.

Le stop et le start ne sont donc plus exécutés dans le même tour GUI. C'est le
comportement qui a été validé séparément sur le TRIGKEY.

`root_pid` dans l'ACK d'une action est le PID observé **au moment de
l'acceptation**, avant l'exécution asynchrone par le Window Manager.

## Reboot et extinction

```powershell
python .\tools\remote\bouchaud-control.py system reboot
python .\tools\remote\bouchaud-control.py system shutdown
```

Ces actions demandent une confirmation interactive, sauf avec `--yes`.
L'ACK est rendu avant l'action physique ; le noyau laisse une courte fenêtre
pour vider la réponse BRDP puis appelle le vrai chemin `kernel::power`.

Pour un reboot entièrement suivi :

```powershell
python .\tools\remote\bouchaud-control.py system reboot --wait-ready
```

Le client prend d'abord un `status` pré-reboot, arme le reboot, puis retente
BRDP jusqu'à obtenir un `status` dont `t_ns` est inférieur à la borne
pré-reboot. Cela prouve un **nouveau boot** et non une reconnexion à l'ancien.

Options :

```text
--ready-timeout 90
--ready-interval 1
```

La télémétrie observée le 25 septembre 2026 a validé le chemin complet : ACK
`cmd=100`, disparition de l'ancien boot, retour d'un heartbeat avec une horloge
réinitialisée, puis `BRDP_ECOUTE port=2222` sur le nouveau noyau.

## Tuer un processus

```powershell
python .\tools\remote\bouchaud-control.py process kill <pid>
python .\tools\remote\bouchaud-control.py process kill-tree <pid>
```

Les PID 0 et 1 sont refusés par le protocole. Les deux commandes demandent une
confirmation sauf `--yes`. `kill-tree` vise la racine et ses descendants ; il
sert aux arbres de services, pas comme substitut à un shell distant.

## Checkpoint de boîte noire

```powershell
python .\tools\remote\bouchaud-control.py checkpoint
```

C'est une action avec effet de bord : elle force un checkpoint. Ne pas la
confondre avec `doctor`, `status` ou les snapshots qui sont des lectures.

## Séquence recommandée d'un essai physique

Console 1 :

```powershell
python .\tools\remote\bouchaud-lab.py telemetry --watch
```

Console 2 :

```powershell
python .\tools\remote\bouchaud-lab.py discover --timeout 10
python .\tools\remote\bouchaud-control.py doctor
python .\tools\remote\bouchaud-control.py browser restart
python .\tools\remote\bouchaud-control.py system reboot --wait-ready
```

En cas d'anomalie, capturer avant de multiplier les actions :

```powershell
python .\tools\remote\bouchaud-lab.py dump --host 169.254.178.21
python .\tools\remote\bouchaud-lab.py serial-capture --host 169.254.178.21
```

## Ce qui a été prouvé physiquement

Sur le TRIGKEY, le 25 septembre 2026 :

- découverte passive UDP : OK ;
- `status` BRDP authentifié : OK ;
- `doctor` multi-commandes : OK ;
- `browser stop` : OK, BRDP et télémétrie restent vivants ;
- `browser start` : OK, BRDP reste vivant ;
- reboot distant : ACK reçu puis reboot matériel réel et retour du nouveau boot ;
- BRDP après reboot : OK après la fenêtre de readiness ;
- le restart monolithique V1.1 a montré une indisponibilité BRDP transitoire ;
  V1.2 le remplace par stop puis start sur deux tours du Window Manager.

Cette liste est un relevé de preuves, pas une promesse générale : toute
évolution du scheduler, de smoltcp ou du cycle de vie des processus doit refaire
les tests concernés.
