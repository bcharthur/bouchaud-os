# Réveil « perdu » entre la demande et la reprise (P13_REVEIL_COEUR_MUET_V1)

Défaut ouvert depuis le 27/10 (« fil noyau endormi jamais réveillé »,
reproduit avant D8, indépendant de D8). Ce document trace la transition
sommeil → Ready → demande CPU → IPI → reprise, nomme le point exact où le
réveil se perd, et donne la reproduction déterministe et le correctif.

## Ce qui n'est PAS perdu

- **Le passage à Ready.** `wake_sleepers` est global : n'importe quel cœur
  qui balaie les échéances (tick, réveil précis armé avant `hlt`, vecteur
  local) rend le dormeur Ready. Le dormeur est bien publié.
- **La mise en file.** `publie_pret` met la tâche dans la file de son cœur
  cible ; la file déduplique, le motif croisé réveilleur/dormeur
  (`BOUCHAUD_C1_REVEIL_PERDU_IDLE_V1`) ferme la fenêtre `IDLE`.

## Où il se perd

| Étape | Code | Ce qui se passe |
|---|---|---|
| Choix du cœur | `publie_pret` → `replace_necessaire` | La tâche est posée sur son cœur **historique**, sauf privilège (`latency_sensitive`) ou tâche *Interactive* face à un cœur hors ordonnanceur. Une tâche *Normale* — dont tous les fils `spawn_noyau` — n'est jamais replacée. |
| Demande CPU | `reveil::decide` | Le cœur historique travaille : `DemandeDifferee` (`need_resched`), **sans IPI**. |
| Reprise | occupant du cœur | La demande n'est servie que par l'occupant : jamais par un cœur **hors ordonnanceur** (shell de l'autorun, contexte de démarrage) avant son retour ; par un **fil noyau** seulement à son point coopératif — jamais s'il masque les interruptions, si l'hôte retire le vCPU, ou s'il n'en atteint pas. |
| Secours | `runnable_steal` | `!task.noyau` : **aucun fil noyau ne se vole**, même déclaré `migrable`. Les cœurs au repos ne peuvent pas le prendre. Seul le fil `console` avait un secours (`secours_tache_coeur_muet`), appelé par son propre chien de garde. |

La tâche est prête, publiée, et n'a aucun chemin vers un processeur.

## Observations qui le montrent

- **Banc de la rafale masquée, TCG SMP2** (matrice console du 27/10) : 2
  démarrages sur 3 en échec `n=0`. Le fil `verrou-suspendu` (normal,
  migrable) est enregistré avec `rq=0` ; le cœur zéro exécute le shell de
  l'autorun, hors ordonnanceur, qui attend **ce fil** en boucle. Le fil n'est
  élu qu'au retour du shell, 30 s plus tard (`CONSOLE_VERROU_SUSPENDU_FAIT
  ... coeur=0`), le cœur un au repos tout du long.
- **Banc du dormeur (diagnostic du 27/10)** : `diag-noyau` occupe cpu2, le fil
  sondeur y attend Ready ; il ne voit jamais la suspension de 3 s.
- **KVM SMP4, run 39667691144** : le fil `console` publié sur le cœur masqué,
  3 s d'attente, chien de garde déclenché — le cas que le secours de la
  console couvrait, et seulement lui.

## Reproduction déterministe

`tools/ci/run_reveil_coeur_muet.sh IMAGE 3000` (commande `serie-dormeur-masque`) :
un fil noyau migrable dort un tick à la fois ; un fil épinglé sur **son**
cœur masque les interruptions 3 s. Témoin compilé du même arbre :
`banc-reveil-historique` (ancienne règle de placement, aucun fil noyau volé).

| TCG | Témoin (règle historique) | Correctif |
|---|---|---|
| SMP2 × 3 | 3/3 rouges : 0 tour, écart 3 001–3 027 ms | 3/3 verts : 776–1 476 tours, écart 7–22 ms |
| SMP4 × 3 | 3/3 rouges : 0 tour, écart 3 001 ms | 3/3 verts : 1 593+ tours, écart ≤ 22 ms |

Aucune durée du banc n'a été modifiée entre les deux bras : même image,
même scénario, seule la règle compilée diffère.

## Protocole retenu : image témoin dédiée (option 2)

Le correctif rend inexécutables, sur l'image qualifiée, les quatre bancs
« cœur du fil » de l'étape console d'os-primitives : ils immobilisent le fil
`console` derrière un cœur masqué pour prouver son secours dédié, et ce fil
est désormais replacé dès qu'un fil noyau occupe son cœur
(`CONSOLE_VERROU_SUSPENDU_HORS_CIBLE`, échec sans faux vert).

- **Noyau livré** : correctif actif par défaut, sans interrupteur
  d'exécution ; aucun contournement accessible.
- **Image témoin** `banc-reveil-historique` : option de compilation de test,
  jamais dans `default`, qui ne rétablit QUE l'ancienne règle de placement au
  réveil et l'exclusion des fils noyau du vol. Construite dans
  `target-temoin` ; os-primitives vérifie que l'image qualifiée garde la même
  empreinte avant et après, qu'elle ne contient pas la bannière
  `BOUCHAUD_IMAGE_TEMOIN`, et journalise SHA source, features et empreinte des
  deux images (`temoin-reveil-historique/identite.txt`). Ses preuves vont
  dans l'artefact séparé `os-primitives-temoin-reveil-historique`.
- Les quatre bancs « cœur du fil » tournent sur cette image, commandes,
  critères et seuils inchangés.
- `run_reveil_coeur_muet.sh` est une étape obligatoire : OK exigé sur l'image
  qualifiée (TCG et KVM, SMP2 et SMP4), ECHEC (code 1) exigé sur le témoin ;
  `NON_QUALIFIANT` (code 2, reproduction non atteinte) n'est jamais vert.

### Résultats TCG (preuves : `docs/p13/preuves/20261027-reveil/`)

| Banc | Image | Résultat |
|---|---|---|
| réveil, SMP2 et SMP4, ×3 | production | 6/6 OK, 761–1 721 tours, écart 6–42 ms, autre cœur au repos vu, 0 double élection, 0 incohérence, fil épinglé jamais volé |
| réveil, SMP2 et SMP4, ×3 | témoin | 6/6 ECHEC attendu, 0 tour, écart 3 001–3 019 ms |
| cœur du fil, secours off / on, ×2 | témoin | 4/4 OK (repli témoin revenu ; relogement par le secours) |
| console verrou suspendu et rafale, SMP2 et SMP4 | production | 4/4 OK, 0 ligne perdue |

## Correctif

1. **Placement** (`reveil::portee_replacement`, pur, testé sur l'hôte) :
   - historique **hors ordonnanceur** → n'importe quel cœur permis, quelle
     que soit la classe (la restriction aux interactives est levée) ;
   - historique tenu par un **fil noyau** (et pas au repos) → un cœur **au
     repos** s'il en existe un ; sinon l'historique, comme avant (déplacer
     vers un autre cœur occupé ne garantit rien de plus) ;
   - occupant utilisateur : inchangé (son quantum le fait céder).
2. **Vol** (`runnable_steal`) : un fil noyau **migrable** se vole comme une
   tâche utilisateur, toujours dans son affinité. Couvre le cas où le cœur
   devient muet après la publication. Les fils noyau historiques (non
   migrables) restent épinglés par leur affinité.

Rien n'est temporisé, aucune borne n'est changée : le correctif ajoute un
chemin vers un processeur là où il n'en existait pas.

## Ce qui protège

- `tools/smp/test_reveil_cible.rs` : 46 tests, dont
  `une_normale_quitte_un_historique_hors_ordonnanceur`,
  `un_historique_tenu_par_un_fil_noyau_cede_la_place_a_un_coeur_au_repos`,
  `portee_du_replacement`. L'assertion qui consacrait le défaut (une normale
  garde un historique hors ordonnanceur) est retirée, avec sa trace.
- `tools/verifie-reveil-coeur-muet.py` : 10 mutations.
- `.github/workflows/p13-cible-kvm.yml` : témoin rouge et correctif vert sous
  KVM SMP2/SMP4.

## Effet attendu sur P13

Le correctif change le placement des tâches *Normales* réveillées derrière un
fil noyau quand un cœur dort, et rend les fils noyau migrables volables. Il
ne peut qu'abréger des attentes Ready → élu ; il doit néanmoins passer la
qualification stricte complète (P2/P4, équité, débit, non-famine) avant
d'entrer dans une baseline : aucun seuil n'est modifié.

## V2 -- deux causes residuelles, attribuees puis corrigees

Run KVM cible 40576409173 (8f472da7) : 1 echec sur 6 en production, KVM SMP2,
`ecart_max_ms=602`. Run d'experience 40582472007 (branche
`exp/reveil-ecart-kvm`, image de production instrumentee, banc seulement) :
**SMP2 8/30 en echec (193 a 420 ms), SMP4 10/10**. Le job etait vert : c'est
la collecte qui reussissait, pas les criteres (`BILAN ok=32 echec=8`).

### Cause 1 -- tache en file sur un coeur muet, aucun coeur au repos (P13_REVEIL_COEUR_MUET_V2)

Preuves (journal du run 40582472007, `preuves/20261027-reveil-v2/experience/kvm-40582472007/`) :

- Dans les 30 essais SMP2, le pire ecart est le PREMIER : il commence 0 a 1 ms
  avant le masquage, dormeur sur le coeur 0 avant, sur le coeur 1 apres. Sa
  duree seule varie : 7-24 ms, 68-78 ms (verts mais anormaux), 193, 365-420 ms.
- Vol de l'hote sur l'intervalle (`MSR_KVM_STEAL_TIME`, `kvm-steal-time=on`) :
  <= 53 us par coeur. **Aucune absence de battement du coeur 1** pendant
  l'ecart : toutes les absences journalisees sont celles du coeur 0, masque
  par le banc. L'hote est exclu ; le coeur 1 elisait.
- Observateur (le masqueur, a chaque tour de sa boucle masquee), essai 19
  (420 ms) : le dormeur est `Ready`, `on_cpu=-1`, **en file sur le coeur 0**
  des t=1 us, et n'en bouge plus ; le coeur 1 n'est **jamais au repos** --
  son occupant est l'indice 6, le fil `console` (videe de son arriere), en
  alternance avec `diag-noyau`, `net-rx-poll`, `usb-hid`,
  `services-metrics`, `bouchaud-telemetrie` ; ses battements arrivent toutes
  les 4 ms.

Chaine : l'echeance du dormeur est servie par le tic du coeur 1 ; a la
publication, l'historique (coeur 0) est tenu par un fil noyau -> portee
`Inactifs` ; aucun coeur au repos (console) -> l'historique est garde ;
`pick_next` ne vole que sur file VIDE, et le coeur 1 en a toujours une. La
tache attend que la console ait fini. En SMP4, un coeur au repos existe
toujours : 13/13.

Sur le coeur 0, les absences classees `Inexpliquee` (masque_us 0-10 pour 3 s)
sont un angle mort de l'instrument, pas un fait du noyau : la section
`without_interrupts` du masqueur n'est pas une section que
`section_juste_avant` mesure ; elle est voulue par le banc.

Correctif : chaque coeur vivant, a son battement (tic du BSP, vecteur local
des AP), reloge les taches pretes en file sur un coeur muet. Regle et
relogement EXISTANTS (`kernel::scheduler::secours`, testes ; appeles jusqu'ici
par le seul chien de garde de la console) : occupant installe depuis plus de
`SECOURS_SEUIL_NS` (20 ms, cinq quanta), tache prete depuis plus longtemps ;
fils noyau epingles jamais deplaces, affinite respectee, coeur d'accueil
vivant et permis, propriete de l'entree par `remove`, barriere puis IPI.
Cas courant : une lecture par coeur (`coeurs_muets`), aucun parcours. Compteur
propre : le `secours` de la console reste le sien. Absent de l'image temoin
`banc-reveil-historique` (qui doit rester rouge).

### Cause 2 -- tache Blocked sans source de reveil (P13_SOMMEIL_SANS_SOURCE_V1)

Vue en cherchant la premiere, en TCG SMP4 (avant ET apres le correctif 1,
1 essai sur ~8) : silence de 0,3 a 2,6 s jusqu'a la fin du masquage, le
dormeur `Blocked`, `on_cpu=-1`, sans echeance. Detecteur par etape
(experience, `preuves/.../experience/tcg-detecteur/`) :
`sorties_bloquees=20 pertes_irq=1 phase=6 pret_us=-1`.

`dort_jusqua` sort aussi de sa boucle sur l'echeance passee sans que le
balayage ait reveille la tache (encore `Blocked`) ; elle effacait l'echeance,
puis ne terminait l'attente qu'apres la ventilation du reveil. Une preemption
ciblee depuis l'interruption (`preempt_from_irq`) dans cette fenetre commutait
une tache `Blocked`, sans echeance, hors de toute file : perdue.

Correctif : terminer l'attente AVANT d'effacer l'echeance ; de meme,
`annule_park_courant` reveille avant d'effacer cle et echeance. Les autres
parkings a echeance n'effacent qu'apres la sortie de `Blocked`.
TCG SMP4 avec les deux correctifs : 20/20, 0 perte, 59 a 134 sorties
bloquees par essai (la fenetre est franchie, sans perte).

### Ce qui protege (V2)

- `tools/smp/test_secours_coeur_muet.rs` : 17 tests (4 nouveaux, `coeurs_muets`).
- `tools/verifie-secours-coeurs-muets.py` (11 mutations),
  `tools/verifie-sommeil-source-reveil.py` (5 mutations) ;
  `verifie-console-asynchrone.py` lit les gardes du relogement la ou il vit.
- `p13-cible-kvm.yml` : banc du reveil sur la livree 10 x SMP2 et 5 x SMP4,
  temoin toujours rouge.

### A/B KVM entrelace (run 40590879833, meme runner)

A = 4dcc3416 (sans correctif), B = 3276302f (les deux correctifs ;
instrumentation de banc identique). Criteres inchanges ; un non-qualifiant ne
compte ni comme succes ni comme echec.

| Bras | SMP2 OK / ECHEC / non qualifiants | SMP4 |
|---|---|---|
| A | 13 / **2** / 5 | 10/10 |
| B | **18 / 0** / 2 | 10/10 |

Les deux non-qualifiants de B (essais 7 et 16), et les cinq de A : le
masqueur est tombe sur le coeur UN ; l'autre coeur est alors le BSP, qui
execute le shell de l'autorun hors ordonnanceur et n'est donc jamais un coeur
electeur au repos (`autres_coeurs_au_repos=0`) -- la condition de reproduction
du banc n'est pas atteinte, `REVEIL_COEUR_MUET_NON_QUALIFIANT
raisons=aucun-autre-coeur-au-repos` (code 2). Dans ces deux essais de B, sans
qu'ils comptent : aucune panique, 0 double election, 0 incoherence `on_cpu`,
fil epingle jamais deplace, 0 perte, ecart max 50 et 49 ms.

### Remesure des non-qualifiants (BANC_TENTATIVES)

Run p13-cible-kvm 40595084969 (487d1b45) : 14 essais qualifiants OK sur 14,
un non-qualifiant (SMP2, meme cause) -- le pas exigeait le code 0 de chaque
repetition, donc rouge. Ce cas tombe 10 a 25 % des demarrages SMP2 : exiger
dix repetitions sans un seul non-qualifiant rendrait le pas rouge sans
defaut. Le protocole existant de `run_ordonnanceur_attribution.sh` est
applique tel quel (`BANC_TENTATIVES=3`) : un NON_QUALIFIANT est remesure, au
plus trois essais, jamais compte ; un ECHEC n'est jamais rejoue ; trois
non-qualifiants de suite restent NON_QUALIFIANT (rouge). Chaque essai est
archive. Utilise par p13-cible-kvm et os-primitives (production et temoin) ;
`verifie-reveil-coeur-muet.py` le verrouille (19 mutations).
