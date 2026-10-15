# Baseline stable Bouchaud OS

**État : NOT_ESTABLISHED** (15 octobre 2026).

Une baseline stable exige la convergence ET la répétabilité : trois campagnes
indépendantes au même HEAD (CI Fast, Integration, Reliability, Ladybird avec
os-primitives TCG+KVM, endurance), P13 qualifié sous KVM à SMP1/2/4, aucun
rouge transverse. Ce n'est pas le cas :

| Condition | Situation au 15 octobre | Source |
|---|---|---|
| Campagne verte au même HEAD | `c7018e56` : Ladybird rouge (A/B IPI, défaut de mesure probable, corrigé `81570c88`, à confirmer) ; endurance rouge (SMP8 NON_QUALIFIANT) | `AUDIT_STABILITY_2026-10-15.md` |
| Répétabilité (3 campagnes same-HEAD) | non faite sur le HEAD courant | — |
| P13 qualifié sous KVM SMP1/2/4 | **non** : TEMOIN_INSTRUMENT aux 4 topologies (« charge sans effet mesurable » ; arrondi des sommeils corrigé, détection au tick ouverte) | run 38645396730 |
| SMP8 | non qualifiable sur runner 4 cœurs | idem |
| Transverses ouverts | `persist-course-probe`, `courant.rs:276`, LibJS `Value.cpp:484` | `P_LOTS_REGISTER.md` |

Ce fichier ne sera réécrit en baseline qu'avec les SHA et runs qui
établissent chacune des conditions ci-dessus.
