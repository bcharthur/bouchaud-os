# Baseline stable — Bouchaud OS (20 octobre 2026)

**NOT_ESTABLISHED.** Voir [P13_CLOSEOUT_2026-10-20.md](P13_CLOSEOUT_2026-10-20.md) puis [P13_CLOSEOUT_2026-10-19.md](P13_CLOSEOUT_2026-10-19.md).

Les commits `7d6c944b` et `cbbd5ec0` (20/10) modifient le chemin de reveil : **toutes les preuves de `f5692b7` / `2582cde9` ci-dessous sont a refaire sur le nouveau SHA** avant toute qualification.

| Critere | Etat / preuve |
|---|---|
| CI Fast, Reliability, Integration, Native IPC, Ladybird au meme SHA | **PASS** sur `f5692b7` : runs 39296905594, 39296905455, 39296905800, 39296905774, 39296905493. Ladybird 20/20 jobs verts |
| Gel global QEMU SMP8 | 4/9 gels >=200ms -> 0/15 apres corrections IRQ/serie; preuve limitee |
| P13 KVM SMP1/2/4 strict | **PARTIEL** : `59e384ce` SMP1 `P13_STRICT_OK` (39387288347) ; SMP2/4 `NON_QUALIFIANT` (charge sans effet) ; a rejouer sur le SHA courant |
| A/B reveil precis KVM SMP1/4 | **NON CONCLUANT** : 0/4 paires qualifies |
| SMP8 KVM dedie >=8 CPU | **NON PROUVE** ; runner `bouchaud-kvm8` necessaire |
| Trois campagnes independantes meme SHA | **NON PROUVEES** |
| Trigkey PHYSICAL, AMD/RTL8168 | **NON PROUVE** |
| GPU composition directe KVM (tuiles) | **NON_CONVAINCANT** : direct p50 58us, copie p50 52us |
| Correction du compteur TSC-deadline strict | `4d279ff` en code, preuve complete same-HEAD a controler |

Un workflow vert avec porte P13 non stricte ne constitue jamais une baseline produit. Interdiction de modifier les seuils ou les quorums pour attribuer du vert. Les anciens rapports restent archives.
