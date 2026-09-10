# H10 — TRIGKEY, machine de reference

Ce document decrit ce que Bouchaud OS sait prouver **sur la machine physique**,
et par quels moyens. Il ne decrit pas ce qu'il devrait savoir faire.

Machine de reference : **TRIGKEY, AMD Ryzen 7 5800H**, 16 coeurs logiques,
~11,5 Gio de RAM utilisable sous UEFI, GOP 1920x1080/32bpp, demarrage UEFI
depuis une cle USB.

---

## 1. Ce que « prouve » veut dire ici

Cinq niveaux, jamais melanges :

| Niveau | Ce qu'il etablit |
|---|---|
| **NON COMMENCE** | Rien de ce qui compte n'est ecrit. |
| **IMPLEMENTE** | Du code existe. Un fichier qui existe n'est pas une preuve. |
| **TESTE QEMU** | Le chemin s'execute sur un noyau reel, machine emulee. |
| **TESTE TRIGKEY** | Le chemin s'execute sur la machine de reference. |
| **NON VALIDE** | Le code existe, personne ne l'a vu tourner la ou il compte. |

**QEMU n'est pas le TRIGKEY.** Un resultat obtenu sous QEMU ne vaut jamais pour
la machine physique, et ce document ne le presente jamais comme tel.

---

## 2. Construire l'image

Depuis Windows, a la racine du depot :

```powershell
git fetch origin feat/trigkey-reference-hardware
git checkout feat/trigkey-reference-hardware
git pull

powershell -ExecutionPolicy Bypass -File .\tools\reference\IMAGE-TRIGKEY.ps1
```

Le script **refuse** de construire sur un arbre modifie : une image qui ne
correspond a aucun commit rend tout releve inexploitable. `-QuandMeme` force,
et etiquette l'image `+sale`.

Trois fichiers sont produits dans `target\reference\`, **a garder ensemble** :

| Fichier | Role |
|---|---|
| `bouchaud-trigkey-stage2-ladybird.img` | l'image a flasher |
| `….img.kernel.elf` | le noyau non strippe, exactement celui embarque |
| `….img.manifeste.json` | commit, SHA256 image + noyau, base, cible, profil |

Flash : **Rufus → mode DD**. Secure Boot desactive, Boot Override sur la cle
UEFI. **Ne jamais choisir le NVMe interne comme cible dans Rufus.**

---

## 3. Les sept commandes

Toutes impriment des marqueurs `H10_*` sur le port serie, que l'enregistreur de
vol conserve. Un releve devient un fichier, comparable au precedent.

| Commande | Ce qu'elle fait |
|---|---|
| `hwinfo` | CPU, mode de quantum local, PCI, NVMe, volumes, USB |
| `hwtest` | six verdicts + bilan ; `passe` / `echec` / `non teste` |
| `bootlog` | le journal d'amorcage |
| `nvmetest` | pilote NVMe : geometrie, statistiques, une lecture reelle |
| `disktest [--ecriture]` | lecture bornee ; ecriture **encadree** (voir §4) |
| `persist-test --pose\|--verifie` | temoin horodate, a relire apres redemarrage |
| `safe-mode [--sortir]` | console texte de secours, bureau non lance |

### `hwtest` : trois etats, pas deux

`NON TESTE` n'est pas un demi-succes, et pas un echec. Une machine sans
controleur USB ne doit pas rendre `echec` sur le point `usb-hid` : elle n'a rien
a dire. Le bilan ne compte **jamais** un `non teste` comme un succes.

---

## 4. L'ecriture disque, et pourquoi elle refuse

Le disque interne du TRIGKEY porte **1 000 215 216 blocs** — un demi-teraoctet,
et le systeme d'exploitation de son proprietaire.

**Aucune ecriture de H10 ne touche le volume brut.** Toutes passent par la
partition portant le GUID de type Bouchaud, et `disktest --ecriture` **refuse**
quand elle n'existe pas :

```
ecriture  REFUSEE : aucune partition Bouchaud sur ce disque.
          Le disque brut porte le systeme de la machine ; ce test
          n'y ecrira jamais.
```

Quand elle existe, l'ecriture porte sur **un seul bloc**, le **dernier** de la
partition, et le contenu d'origine est **relu avant** puis **repose apres**. Un
test qui laisse le disque modifie n'est pas un test.

### Le defaut que cette regle a deja attrape

La premiere version demandait `bloc::present(Volume::DONNEES)`. Le pilote ATA
enregistre son disque esclave sur **ce meme volume**, a l'amorcage et sans
condition : la reponse etait donc vraie sur un disque vierge, et l'ecriture
avait lieu — `lba=8211`, sur soixante-quatre mebioctets qui auraient pu etre
autre chose.

C'est le scenario de validation qui l'a trouve, pas une relecture. La barriere
repose maintenant sur `installation::systeme_monte()`, qui n'est vrai que si une
partition de type Bouchaud a ete **trouvee et montee**.

---

## 5. Le test de persistance

```
persist-test --pose      # ecrit un temoin horodate, synchronise
<REDEMARRER>
persist-test --verifie   # relit
```

`--verifie` lance **dans la meme session** que `--pose` rend
`verdict=non-concluant raison=meme-session`, et le dit :

> CETTE RELECTURE NE PROUVE RIEN : le temoin a ete pose dans cette meme
> session, et cette commande relit la MEMOIRE.

Seul un redemarrage entre les deux fait de la relecture une preuve.

`--pose` distingue aussi un echec de synchronisation (`code=-1`) d'une zone
vide (`0`) : une panne d'ecriture ne passe pas pour un succes.

---

## 6. Matrice QEMU / TRIGKEY

Etat au commit de cette branche. **Aucune ligne n'est marquee TESTE TRIGKEY :
depuis le checkpoint physique, aucun essai n'a ete conduit avec cet outillage.**

| Point du contrat | QEMU | TRIGKEY | Preuve |
|---|---|---|---|
| 1. Boot reproductible | ✅ | ⚠️ non valide | image + SHA256 + manifeste ; boot physique observe avant H10 |
| 2. Journal exploitable | ✅ | ✅ | BLACKBOX : 102 enregistrements, 41 Ko de serie, 2 446 evenements de vol releves le 10/09 |
| 3. Mode texte de secours | ✅ | ⚠️ non valide | `H10_SAFEMODE etat=actif` sous QEMU ; jamais declenche sur la machine |
| 4. Detection PCI | ✅ | ✅ | `hwinfo` ; `[PCI-NG] peripheriques=…` releve physiquement |
| 5. Detection NVMe | ✅ | ✅ | `BOUCHAUD_NVME_GREEN … blocs=1000215216` releve physiquement |
| 6. Volume bloc NVMe | ✅ | ✅ | `H10_HWINFO_VOLUME` ; volume publie physiquement |
| 7. Lecture bloc | ✅ | ✅ | `H10_NVMETEST verdict=ok` ; `NVME_IO_CQE_OK` releve physiquement |
| 8. Ecriture encadree | ✅ | ⚠️ non valide | `relecture=1 restaure=1` sous QEMU ; refus verifie sur disque vierge |
| 9. Persistance apres reboot | ⚠️ partiel | ⚠️ non valide | sous QEMU la zone de persistance est absente (`code=-1`) ; le cycle complet exige la machine |
| 10. Sept commandes shell | ✅ | ⚠️ non valide | `TRIGKEY_H10_OK` |
| 11. Aucun chantier hors perimetre | ✅ | — | ni navigateur, ni GUI avancee, ni reclaim memoire dans cette branche |
| 12. Pas de merge sans preuve physique | — | ⚠️ **en attente** | voir §8 |

### Ce que le TRIGKEY a deja montre, hors H10

Releve du 10 septembre 2026, session `20260910222310672` :

- **`fatal_records=0`** — le systeme ne faute pas, il gele. Ce n'est pas le meme
  defaut, et ce n'est pas la meme recherche.
- **Un seul coeur battait.** 16 coeurs en ligne (`SMP4_AP_STARTED count=15`),
  2 446 evenements de vol **tous `cpu=0`**, `timer1/2/3` a zero sur chaque
  echantillon. Cause : seul TSC-deadline etait implemente, or c'est une
  fonctionnalite Intel absente du Ryzen. **Corrige** (repli LAPIC periodique),
  **mesure sous QEMU** (`battants=4 en_ligne=4`), **non valide sur TRIGKEY**.
- **L'entree etait lue a la cadence du rendu.** `FPS: 0` puis `FPS: 1` pendant
  cinq secondes au demarrage du bureau. Diagnostic confirme ; le remede a ete
  **retire** parce qu'il cassait le demarrage des programmes utilisateur.

---

## 7. Valider en local

```bash
tools/ci/run_trigkey_h10.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin
```

Le scenario lance les sept commandes sur un disque **avec** partition Bouchaud,
puis relance `disktest --ecriture` sur un disque **vierge** pour verifier que
l'ecriture est refusee. La seconde moitie est la plus importante.

Verdict attendu : `TRIGKEY_H10_OK`.

### Exemple de log attendu

```
H10_HWINFO_TIMER mode=lapic-periodique lapic_hz=62388078
H10_HWTEST point=smp-battement    verdict=passe      detail=tous les coeurs recoivent leur quantum
H10_HWTEST point=timer-local      verdict=passe      detail=lapic-periodique
H10_HWTEST point=nvme-present     verdict=passe      detail=en service
H10_HWTEST point=bloc-lecture     verdict=passe      detail=le premier bloc du disque a ete lu
H10_HWTEST point=partition-bouchaud verdict=passe    detail=presente : l'ecriture encadree est possible
H10_HWTEST point=usb-hid          verdict=non-teste  detail=aucun controleur xHCI sur ce bus
H10_HWTEST_BILAN passes=5 echecs=0 non_testes=1 verdict=ok
H10_NVMETEST verdict=ok blocs=131072 taille_bloc=512 tete=31c08ed8
H10_DISKTEST verdict=ok phase=ecriture lba=131004 relecture=1 restaure=1
H10_PERSIST verdict=non-concluant phase=verifie raison=meme-session
H10_SAFEMODE etat=actif miroir_serie=1
```

Sur un disque sans partition Bouchaud :

```
H10_DISKTEST verdict=refuse phase=ecriture raison=pas-de-partition-bouchaud
```

---

## 8. La sequence physique a conduire

C'est ce qui manque pour clore H10. Aucun de ces resultats n'existe encore.

1. Construire l'image (§2), **noter le commit et le SHA256**.
2. Flasher, demarrer, laisser arriver au bureau.
3. `hwinfo` — relever le mode de quantum et le nombre de coeurs.
4. `hwtest` — **le point qui compte** : `smp-battement` doit rendre `passe`
   avec seize coeurs. C'est la validation du correctif du timer.
5. `nvmetest`, puis `disktest` (lecture seule d'abord).
6. `disktest --ecriture` — **doit REFUSER** si aucune partition Bouchaud n'est
   installee. Un refus est le bon resultat.
7. `persist-test --pose`, **redemarrer**, `persist-test --verifie`.
8. `safe-mode`, redemarrer, verifier que le bureau ne se lance pas, puis
   `safe-mode --sortir`.
9. Debrancher la cle, extraire la BLACKBOX, renvoyer l'archive.

**A renvoyer** : l'archive BLACKBOX, le commit, le SHA256 de l'image, et le
manifeste. Le manifeste est ce qui permet de resoudre une adresse en
`fonction + fichier:ligne` — sans lui, un RIP releve ne vaut rien.

---

## 9. Implemente / teste / non teste / risques

### Implemente et teste sous QEMU
- Les sept commandes, verdicts compris.
- Le refus d'ecriture hors partition Bouchaud, **verifie dans les deux sens**.
- Le timer local par coeur, mode periodique, avec mesure du battement.
- Le mode de secours, active et desactive.

### Implemente, NON TESTE sur TRIGKEY
- **Tout ce qui precede.** Aucun essai physique n'a ete conduit avec cet
  outillage.
- En particulier le battement des **seize** coeurs : la mesure existante porte
  sur quatre coeurs emules.

### Non teste nulle part
- Le cycle complet de persistance avec redemarrage : la zone de persistance est
  absente des disques QEMU utilises, et `--pose` y rend `code=-1`.

### Risques restants
- **Le gel du bureau n'est pas corrige.** Son diagnostic est etabli — l'entree
  lue a la cadence du rendu — et le remede a ete retire parce qu'il cassait le
  demarrage des programmes utilisateur. Le defaut sous-jacent, une tache noyau
  perpetuelle qui interfere avec `exec`, reste **ouvert**.
- **Le clavier passe par le repli EP0.** Sur la session physique, `kbd=6` contre
  `mouse=556` : l'Interrupt-IN du clavier ne remonte quasiment rien, et le repli
  le masque au lieu de l'expliquer.
- **La calibration du timer local** est faite une fois, sur un coeur, et
  supposee valable pour tous. Vrai pour les coeurs d'un meme paquet ; non
  verifie sur une machine multi-paquets.
- **`disktest --ecriture` ecrit sur le dernier bloc de la partition.** Si un
  systeme de fichiers y placait des donnees, elles seraient reposees a
  l'identique — mais une coupure entre l'ecriture et la restauration les
  perdrait.
