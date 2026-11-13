# État courant

**Source de vérité : [status.json](status.json).** Ce fichier ne recopie ni SHA ni verdict ; il dit où les lire et ce qu'ils signifient.

| Question | Où |
|---|---|
| Quel `main`, à quelle date ? | `status.json` → `observed_main_head`, `as_of` |
| Y a-t-il un candidat de qualification ? | `status.json` → `candidate` (aucun depuis la fusion de la PR #241) |
| Quelles PR sont ouvertes, empilées ou non ? | `status.json` → `open_prs` ; **rien de ce qu'elles contiennent n'est sur `main`** |
| Les workflows du push de `main` | `status.json` → `ci.push_main_*` |
| Les workflows d'un SHA de branche qualifié | `status.json` → `ci.push_<sha>`, `ci.pr*` ; le SHA final d'une PR et ses checks : la PR elle-même |
| Verdicts sécurité de la chaîne de mise à jour | `status.json` → `atomic_platform.security_verdicts` (+ `security_details`) |
| Verdicts fonctionnels A/B, recovery, persistance, services | `status.json` → `atomic_platform.functional_verdicts` |
| Niveau de preuve | `status.json` → `atomic_platform.proof_levels` : **HOST + TCG ; ni KVM ni PHYSICAL** |
| P13 | `status.json` → `p13` : SMP1/2/4 PASS ; SMP8 `PENDING_DEDICATED_KVM8` ; final `BLOCKED_BY_RUNNER_PROVISIONING` |
| Protection de `main` | `status.json` → `main_protection` = `BLOCKED_BY_HUMAN_GITHUB_SETTING` |
| PHYSICAL | `status.json` → `physical` = `NOT_STARTED` ([TRIGKEY_CAMPAIGN.md](TRIGKEY_CAMPAIGN.md)) |
| Limites et bloquants | `status.json` → `limitations`, `known_blockers` ; détail et priorités : [KNOWN_LIMITATIONS.md](KNOWN_LIMITATIONS.md) |
| Anomalies ouvertes | `status.json` → `open_anomalies` |

## Lire un verdict

- `PASS` : la preuve nommée a tourné et est verte, au niveau indiqué (HOST = tests hôte, TCG = QEMU émulé). Jamais extrapolé à KVM ni au matériel.
- `NOT_IMPLEMENTED` : le mécanisme n'existe pas (clé de production, anti-rollback) ; c'est voulu et documenté dans [UPDATE_RECOVERY.md](UPDATE_RECOVERY.md).
- `NON_QUALIFYING` : un échec existe et sa cause n'est pas établie ; il n'est ni effacé ni requalifié en infra sans preuve (SIGCHLD multi-fils, voir ci-dessous).
- `BLOCKED_*` : action humaine requise.

## SIGCHLD multi-fils (run 41648148378, `main@992e67e1`)

`p13-primitives-noyau / primitives / probes` : `SIGCHLD_MULTIFIL_ECHEC n=1` dans la passe TCG ; les trois passes KVM du même job sont 8/8.

- **Signature temporelle** : environ 14 s entre la sonde précédente et l'échec, contre environ 4 s pour une passe saine. Un cas a donc atteint sa borne de 10 s : un réveil SIGCHLD sans fils récoltable, ou aucun zombie publié.
- **Artefact** du run : illisible depuis la session (proxy 403 sur l'hôte de blobs).
- **Reproductions ciblées** sur le noyau exact `21fca026` : `tools/ci/run_sigchld_repetition.sh`. 30 exécutions isolées en SMP4, 30 sous `strace echecs` à chacun de SMP1, SMP2 et SMP4, et 5 rejeux du banc `run_os_primitives.sh` complet, soit **125 exécutions, 0 échec**.
- **Code relu** : zombie posé avant SIGCHLD sur les deux chemins (`exit_current`, `tue_processus`), récolte unique sous `PROCESS_TABLE`.
- **Hypothèse réfutée par l'occurrence 2** : un fil condamné qui mourrait sans décrémenter `threads`, et le processus ne deviendrait jamais zombie.
- **Occurrence 2** (`main@b3d4077`, job 131536326101, passe TCG ; les trois passes KVM sont vertes ; suivi dans **#311**) :
  - le cas fautif est `boucle` ;
  - le fil qui attend n'est réveillé qu'à 10 s, par la borne du test (`EINTR`), avec `sigchld_ms=-1` ;
  - l'enfant était pourtant **zombie et récoltable** (`waitpid=60`).
  - Ce qui manque est donc la **livraison de SIGCHLD au parent** quand l'un de ses fils tourne en boucle, sous TCG.
  - Même arbre vert au SHA 85cd1af. **Non résolu.**
- **Diagnostic ajouté** : `run_os_primitives.sh` publie désormais dans le journal du job le cas fautif, son réveil, et les sorties et morts des fils de la racine en échec. Une prochaine occurrence sera attribuable sans l'artefact.

Si ce même échec réapparaît sur la PR : **non prêt à fusionner** tant qu'il n'est pas classé.
