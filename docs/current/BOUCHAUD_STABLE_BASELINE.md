# Baseline stable — Bouchaud OS (20 octobre 2026)

**NOT_ESTABLISHED.** Voir [P13_CLOSEOUT_2026-10-20.md](P13_CLOSEOUT_2026-10-20.md) puis [P13_CLOSEOUT_2026-10-19.md](P13_CLOSEOUT_2026-10-19.md).

Les commits `7d6c944b` et `cbbd5ec0` (20/10) modifient le chemin de reveil : **toutes les preuves de `f5692b7` / `2582cde9` ci-dessous sont a refaire sur le nouveau SHA** avant toute qualification.

| Critere | Etat / preuve |
|---|---|
| CI Fast, Reliability, Integration, Native IPC, Ladybird au meme SHA | **PASS** sur `d097281d` (20/10) : CI Fast, Integration, Reliability V3, Native IPC, Security Runtime, SCM_RIGHTS, ladybird-sync verts ; Ladybird push 39423953091 **20/20** (verdict de convergence compris). Precedent : `f5692b7` |
| Gel global QEMU SMP8 | 4/9 gels >=200ms -> 0/15 apres corrections IRQ/serie; preuve limitee |
| P13 KVM SMP1/2/4 strict | **PARTIEL** : SMP2 `P13_STRICT_OK` sur `d097281d` et `d02e8294` ; SMP1 2/2 puis 1/2 ; SMP4 0/2 puis 1/2 -- echecs restants : emission serie masquee (decision humaine, closeout §6.4) |
| Ladybird au meme SHA `ed5602f4` | **Ladybird 2/2 runs verts** (PR 39479638849 et push 39479633757 : 16/16 jobs Ladybird chacun, dont diagnostic memoire -- rouge 2/2 sur `d02e8294`, `f7ecba30` POLLOUT entre les deux) ; convergence du run push rouge par `primitives` seulement (A/B et attribution P13) ; baseline **NOT_ESTABLISHED** (P13 non reproduit) |
| Ladybird au meme SHA `d02e8294` | **FAIL** (push 39447642999) : jobs Ladybird reels verts, mais diagnostic memoire rouge 2/2 runs (correctif candidat `f7ecba30`, a mesurer) et primitives rouges ; baseline **NOT_ESTABLISHED** |
| A/B reveil precis KVM SMP1/4 | **NON CONCLUANT** par construction (bras `off` = le defaut corrige ; decision humaine closeout §6.1) ; bras `on` 8/8 `QUALIFIE_OK` sur `d097281d` |
| SMP8 KVM dedie >=8 CPU | **NON PROUVE** ; runner `bouchaud-kvm8` necessaire |
| Trois campagnes independantes meme SHA | **NON PROUVEES** |
| Trigkey PHYSICAL, AMD/RTL8168 | **NON PROUVE** |
| GPU composition directe KVM (tuiles) | **NON_CONVAINCANT** : direct p50 58us, copie p50 52us |
| Correction du compteur TSC-deadline strict | `4d279ff` en code, preuve complete same-HEAD a controler |

Un workflow vert avec porte P13 non stricte ne constitue jamais une baseline produit. Interdiction de modifier les seuils ou les quorums pour attribuer du vert. Les anciens rapports restent archives.
