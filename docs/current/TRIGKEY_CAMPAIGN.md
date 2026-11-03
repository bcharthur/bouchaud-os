# Campagne PHYSICAL Trigkey Speed S5 — protocole exact

**Statut : PHYSICAL_PENDING** — aucune étape ci-dessous n'a été exécutée physiquement dans la session du 2026-11-03. Ce document n'affirme rien sur le matériel ; il dit comment le prouver.

## 0. Image et provenance (obligatoire avant de brancher la clé)

```bash
git switch <SHA candidat> && git status --porcelain   # doit être vide
tools/release/build_trigkey_image.sh out/trigkey [ramdisk-ladybird.img]
sha256sum -c out/trigkey/bouchaud-trigkey.img.sha256
```

Conserver **ensemble** : `bouchaud-trigkey.img`, `.sha256`, `kernel.elf` (non strippé), `manifest.json` (commit, SHA Ladybird `third_party/UPSTREAM.md`, toolchain, fonctionnalités). Flash : mode DD (Rufus/`dd`), Secure Boot désactivé, Boot Override sur la clé. **Jamais** le NVMe interne comme cible du flash. Une campagne = un `manifest.json` ; ne pas mélanger deux images.

Capture : console série si câble disponible, sinon boîte noire sur la clé (`tools/reference/extract-blackbox.py --image <clé>`), photos d'écran horodatées pour les étapes visuelles. Symbolisation d'une panne : `tools/reference/symbolise-blackbox.py` avec **ce** `kernel.elf` (le manifeste est vérifié).

## 1. Identification (lève l'incohérence 5700U/5800H)

`BOUCHAUD_HWPROBE_CPUID brand="…" family=… model=… stepping=… eax=…` puis le rapport `/diagnostics` (PCI, mémoire, GOP).
Attendu 5700U : family `0x17`, model `0x68`. Attendu 5800H : family `0x19`, model `0x50`. Recopier la ligne telle quelle dans `TRIGKEY_REFERENCE.md` ; c'est elle qui fait foi, pas l'étiquette commerciale.

Relever aussi : RAM utilisable, liste PCI (NVMe, RTL8168 `10ec:8168`, xHCI, HDA `class=04:03`, iGPU `class=03`), mode GOP courant.

## 2. Étapes et critères

| # | Étape | Preuve exigée (marqueur ou observation) | Échec si |
|---|---|---|---|
| 1 | Boot UEFI | `BOUCHAUD_BOOT_POINT` jusqu'au bureau | arrêt, reboot, triple fault |
| 2 | SMP | `SMP4_AP_STARTED count=15 expected=15`, `SMP_NG2_SCHEDULER` | < 15 AP |
| 3 | GOP | `BOUCHAUD_HWPROBE_DISPLAY current=WxH` = résolution écran | image corrompue |
| 4 | Clavier | `BOUCHAUD_USB_HID_FIL_LANCE` + saisie terminal | touche perdue/répétée |
| 5 | Souris | `[USB-HID-POINT]` + déplacement fluide | gel du pointeur |
| 6 | Hotplug USB | débrancher/rebrancher clavier : reprise sans reboot | périphérique perdu |
| 7–9 | NVMe read/write/flush | `BOUCHAUD_NVME_GREEN`, `NVME_IO_CQE_OK`, écrire un fichier, `sync` | erreur CQE, timeout |
| 10 | Reboot/persistance | le fichier de l'étape 8 relu identique après reboot | contenu absent/différent |
| 11 | RTL8168 | `BOUCHAUD_TRIGKEY_RTL8168_DETECTED`, `…_LINK_UP` | pas de lien câble branché |
| 12–14 | DHCP / DNS / TCP | bail obtenu, résolution d'un nom, connexion TCP | timeout |
| 15 | TLS | page HTTPS chargée (certificat validé) | erreur TLS |
| 16 | Desktop | bureau interactif 5 min sans gel | gel > 1 s |
| 17 | Ladybird | `BOUCHAUD_STAGE2_LADYBIRD_RUNTIME_OK`, fenêtre navigateur | crash au lancement |
| 18 | Example Domain | `https://example.com` rendu | page blanche |
| 19 | Wikipedia | page article rendue, images | blocage |
| 20–21 | Scroll / clic | défilement molette, lien suivi | saccade > 1 s, clic perdu |
| 22 | Multi-onglets | 5 onglets cross-site | crash |
| 23 | Process swaps | navigation cross-site (nouveau WebContent) | ancien process non libéré |
| 24 | Worker | page avec Web Worker | worker bloqué |
| 25 | Sandbox | `NNP_ABSENT` absent du journal ; profils de rendu confinés | rendu non confiné |
| 26 | Crash/recovery navigateur | tuer un WebContent : onglet récupéré, bureau vivant | bureau gelé, panique |
| 27 | Persistance navigateur | cache/historique relus après redémarrage du navigateur | perte |
| 28 | Reboot | redémarrage propre | blocage à l'extinction |
| 29 | Endurance | 60 min navigation + défilement, aucune panique | panique, gel, fuite mémoire croissante |
| 30 | Blackbox recovery | extraire la boîte noire de la clé et la symboliser avec `kernel.elf` | illisible / non symbolisable |

**Machine figée sans panique = FAIL.** Une étape non exécutée = `NOT_RUN`, jamais `PASS`.

## 3. Rapport

Remplir `docs/current/TRIGKEY_CRITERIA_EXAMPLE.json` (copie datée) avec : `manifest.json` (copié), résultat par étape (`PASS/FAIL/NOT_RUN` + preuve), ligne CPUID, journaux/boîte noire en artefacts. Mettre à jour `status.json` → `last_physical` uniquement avec un rapport complet.
