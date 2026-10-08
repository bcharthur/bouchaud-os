# P1 → P13

| Lot | Sujet | État |
|---|---|---|
| P1 | UI Bouchaud / BrowserHost | DONE / QEMU |
| P2 | Compositor → WM / lifecycle | PARTIAL, proche fermeture |
| P3 | damage tracking | DONE / QEMU |
| P4 | scroll asynchrone | DONE / QEMU |
| P5 | WebWorker | DONE / QEMU |
| P6 | sandbox / rôles / NNP | DONE / HOST+QEMU |
| P7 | profil / SQL / cache | DONE / QEMU |
| P8 | audio | PARTIAL, QEMU AC'97 seulement |
| P9 | multi-WebContent / isolation / OOPIF | PARTIAL, proche fermeture |
| P10 | GPU | BLOCKED, fondation virtio commencée |
| P11 | WPT | DONE sur corpus courant |
| P12 | documentation | DONE pour cette synthèse |
| P13 | convergence | PARTIAL |
| P18 | `/proc` / comptabilité historique | DONE ciblé |

## P2/P9
La croissance mémoire linéaire a disparu mais le banc doit être rejoué jusqu'à
20/20 swaps avec plateau confirmé.

## P10
Transport virtio-PCI moderne + `GET_DISPLAY_INFO`. Pas de scanout GPU 2D
complet, 3D, Vulkan, Skia GPU ou pilote AMD physique.

## P13
Bloqué par le premier bras KVM A/B. La performance elle-même est verte.
