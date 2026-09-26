# P13 — Ladybird convergence gate

Ce lot ne remplace pas les mecanismes deja presents au HEAD `0500fc80`.
Il les relie et les rend difficiles a regresser.

## Ce qui est corrige

- **CI Fast / Reliability** : la table de contrat du faux serveur BRDP connait
  maintenant `services-page`, capture serie, internet proof et Remote Control.
- **Vrai artefact Ladybird** : le workflow canonique tourne aussi sur
  `claude/**` et `feat/**`, genere un manifeste SHA-256 lie au HEAD Bouchaud et
  au SHA Ladybird epingle, puis le reverifie apres telechargement.
- **WebWorker / codecs / JS / rendu** : un verdict unique exige HTTP worker,
  blob worker, 11/11 codecs, JS, handshake GUI et surface presentee.
- **P4** : le sampler reste hors de xHCI et le recyclage des instances
  `browser.*.<pid>` reste obligatoire et teste.
- **P18** : `/proc/stat`, `/proc/self/stat` et `/proc/<pid>/stat` dynamiques sont
  verrouilles par un garde-fou source.
- **startup / IPC** : les marqueurs de demarrage Ladybird sont relies au smoke,
  et le tranchant IPC ring3 existant reste une preuve separee obligatoire.
- **stockage / cache** : le page-cache publie maintenant son activite et son
  pire temps d'acquisition dans Services, sans ajouter de verrou d'observation.
- **rendu / GPU** : le backend GPU reel est publie dans Services. Si aucun
  backend GPU n'est enregistre, il est annonce `indisponible` ; le bureau n'est
  pas declare en panne pour autant.

## Ce qui n'est PAS pretendu

- Pas de pilote 3D materiel invente : le backend actuel reste CPU raster +
  linear scanout quand BGA est actif.
- Pas de declaration artificielle « P2/P3 complet » : le lot force le vrai
  build canonique et ses preuves ; les capacites P2/P3 de la roadmap restent
  guidees par leurs tests fonctionnels.
- Aucun changement RTL8168/DHCP/DNS/TCP/TLS.

## Debug physique

Apres boot d'une image de laboratoire avec `BOUCHAUD_DEBUG_TOKEN` :

```powershell
.\tools\remote\collect-ladybird-debug.ps1
```

Le script produit `target\ladybird-debug-*.zip` avec doctor, Services complet,
journal serie, evenements et dump BRDP. C'est le bundle a joindre au prochain
debug.
