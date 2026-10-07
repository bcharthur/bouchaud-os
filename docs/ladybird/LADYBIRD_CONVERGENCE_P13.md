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

## État au 2026-10-07

Le verdict `BOUCHAUD_LADYBIRD_CONVERGENCE_OK` n'est **pas** atteint. Sur
`90c34e49` (run 37627107473) :

| Banc | Résultat |
|---|---|
| browser-host smoke, performance, sites réels, ordre worker | vert |
| WPT | `LADYBIRD_WPT_OK` — 50 fichiers, 4632/4641 (Linux 4628/4641), égaux 49, mieux 1, moins 0, échéances 0, 584 s |
| robustesse | `LADYBIRD_CRASH_RENDU_OK`, `LADYBIRD_OOPIF_OK` ; cycle des workers **ECHEC** (3 workers non recoltés par le navigateur) |
| endurance 10 min (TCG) | **ECHEC** : 29 cycles (≥ 60 exigés), 6 cadres > 30 s ; aucun crash, un seul Compositor |
| endurance KVM (diagnostic) | **ECHEC** |

Cause du cycle des workers trouvée et corrigée dans le noyau (`eea4b8a7`,
masque des signaux par fil) : le navigateur ne voyait que la première mort
d'un fils. Preuve locale (TCG) ; preuve Ladybird attendue au run suivant.
Sous KVM, l'horloge monotone avançait 4,5 fois trop vite (`b94542b7`) et le
disque écrivait en PIO (`fe049123`) : les mesures KVM antérieures à ces
commits ne sont pas comparables.

Le budget d'endurance n'est **pas** changé : il ne le sera que sur mesures
écrites (historique, avant/après, KVM contre TCG, temps par étape).

### Mise à jour — run 37654172489 (`9d785c89`)

| Banc | Résultat |
|---|---|
| robustesse | **`LADYBIRD_WORKER_CYCLE_OK`** (crash rendu et OOPIF toujours OK) |
| endurance 10 min (TCG) | ECHEC : 29 cycles, 6 cadres > 30 s, 26/29 workers ; WebWorker **29 créés / 29 récoltés** (1/29 avant) ; 3 `NNP_ABSENT` |
| cache et SQL | ECHEC : **KERNEL PANIC** `snapshot.rs:66` → corrigé en `d75d2b3b` |
| endurance KVM | ECHEC : 13 cycles (≥ 30), 2 cadres > 30 s ; aucune panique ; horloge de l'invite ~1,6× trop rapide |
| WPT, sites, smoke, performance, ordre worker | vert |

Les trois `NNP_ABSENT` ont désigné une course d'héritage fork/exec (un
WebWorker avec le profil du courtier), corrigée en `7649056a`.
