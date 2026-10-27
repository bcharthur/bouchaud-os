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
