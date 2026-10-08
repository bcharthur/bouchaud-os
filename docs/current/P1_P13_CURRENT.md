# P1 → P13

| Lot | Sujet | État |
|---|---|---|
| P1 | UI Bouchaud / BrowserHost | DONE / QEMU |
| P2 | Compositor → WM / lifecycle | PARTIAL, preuve mémoire incomplète |
| P3 | damage tracking | DONE / QEMU |
| P4 | scroll asynchrone | DONE / QEMU |
| P5 | WebWorker | DONE / QEMU |
| P6 | sandbox / rôles / NNP | DONE / HOST+QEMU |
| P7 | profil / SQL / cache | DONE / QEMU |
| P8 | audio | PARTIAL, QEMU AC'97 seulement |
| P9 | multi-WebContent / isolation / OOPIF | PARTIAL, 20/20 swaps prouvés, mémoire ouverte |
| P10 | GPU | BLOCKED, fondation virtio commencée |
| P11 | WPT | DONE sur corpus courant |
| P12 | documentation | DONE pour cette synthèse |
| P13 | convergence | PARTIAL |
| P18 | `/proc` / comptabilité historique | DONE ciblé |

## P2/P9
Deux répétitions du run 37800200518 au HEAD technique `3658af0` prouvent chacune
20/20 swaps, 20 fermetures et 20 WebContent enfants collectés. Le Compositor garde
2 contextes et 4623 KiB de surfaces aux checkpoints M1/M2, mais le RSS du
WebContent principal gagne encore 11412 / 11456 KiB en seconde phase. M0 n'a
pas encore les trois relevés stables requis. P2/P9 n'est pas DONE.
Voir [le handoff et les mesures](HANDOFF_CODEX_2026-10-08.md).

## P10
Transport virtio-PCI moderne + `GET_DISPLAY_INFO`. Pas de scanout GPU 2D
complet, 3D, Vulkan, Skia GPU ou pilote AMD physique.

## P13
La campagne complète du HEAD `3658af04f05e04edf732cc086ddc890faaf88c52` est rouge :
Compositor réellement remplacé (PID 18 → 28, sortie 0) sur le bras KVM A,
banc mémoire rouge, et pile os-primitives non symbolisable après corruption série.
Le faux comptage historique à zéro a été diagnostiqué et remplacé par une preuve
cumulative qui détecte maintenant ce vrai remplacement. Les performances et
les endurances TCG 10 min / KVM 20 min sont vertes. Aucun passage à P10.
