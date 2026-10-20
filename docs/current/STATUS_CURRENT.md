# Etat courant Bouchaud OS — 20 octobre 2026

**P13: PARTIAL. Baseline: NOT_ESTABLISHED.** Reference technique : [P13_CLOSEOUT_2026-10-20.md](P13_CLOSEOUT_2026-10-20.md) (causes racines du `NON_QUALIFIANT` KVM), puis [P13_CLOSEOUT_2026-10-19.md](P13_CLOSEOUT_2026-10-19.md).

- **20/10 — cause du `NON_QUALIFIANT` trouvee dans le code** (run 39308093631) : au repos, le coeur reveillait plus tard que charge (KVM SMP1 p95 917 us contre 212 us). Deux defauts corriges : echeance due ou a < 20 us non armee puis `hlt` jusqu'au tic (`cbbd5ec0`, BOUCHAUD_REVEIL_DU_V1) ; tache bloquee restee installee sur un coeur en `hlt` jamais reveillee par IPI (`7d6c944b`, BOUCHAUD_REVEIL_TACHE_INSTALLEE_V1). Preuves HOST + TCG (futex-pingpong SMP4 2 857 -> 95 us, SMP2 ~974 -> ~69 us, invariants du banc tenus).
- **KVM sur `59e384ce`** (strict 39387288347 + PR 39387289004) : repos p95 SMP1 851-955 -> 134-144 us, SMP4 177-416 -> 140-155 us ; **attribution KVM SMP1 `P13_STRICT_OK`** (premiere fois) ; SMP2/4 `NON_QUALIFIANT` « charge sans effet » (repos ~ charge ~ 150-180 us) ; SMP8 contamine (steal). A/B : bras `on` 8/8 QUALIFIE_OK, bras `off` impossibles a qualifier par construction (decision humaine, voir P13_CLOSEOUT_2026-10-20 §5). Deux QUALIFIE_ECHEC SMP4 a elucider. Ladybird 20/20 jobs verts (pas de regression). `2f2868c5` (echeance propre, bascule synchrone, diag sans rdmsr) : HOST+TCG, KVM a mesurer.

- Branche: `claude/ladybird-observability-performance`. Ne pas merger vers `main` avant qualification; [PR #226](https://github.com/bcharthur/bouchaud-os/pull/226) en brouillon.
- Derniere campagne entiere sur un meme SHA connue verte: `f5692b70ea55349c0ac3e0beb5c8653a65b869ad` (CI Fast, Reliability V3, Integration, Native IPC Runtime, Ladybird; Ladybird 20 jobs verts).
- Analyseur strict P13 corrige avec `6bf0727` et `4d279ff` : le 17e relevé de retour a la politique par defaut ne compte plus dans les deux releves de chaque bras. Pas de seuil baisse.
- KVM P13 attribution SMP1/2/4: `NON_QUALIFIANT`; A/B reveil precis SMP1/4: 0/4 tours qualifiés, `NON_CONCLUANT`. Les jobs peuvent etre verts sous `P13_STRICT: 0` sans qualifier P13.
- Sonde gel, diagnostics hors hard IRQ et jeton serie deja integres: mesure QEMU SMP8 4/9 gels >=200ms avant vs 0/15 apres correctifs; ce n'est pas une garantie physique.
- GPU P10+ direct/composition: banc tuiles KVM `NON_CONVAINCANT` dans 39296905493 (direct 58 us, copie 52 us).
- Manquants: P13 strict sur hote KVM propre, SMP8 KVM >=8 CPU, trois campagnes au meme SHA, validations Trigkey.
- Aucune nouvelle phase EEVDF/tickless, conteneurisation ou IA a substituer a la fermeture stabilite/performance.

Les anciens etats de `3658af04` restent disponibles dans l'historique Git, mais ne sont plus le statut courant.
