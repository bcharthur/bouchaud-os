# TRIGKEY Speed S5 — référence physique actuelle

La TRIGKEY Speed S5 est la **machine physique de référence** de Bouchaud OS.

Ce document sépare volontairement la configuration de référence déclarée des
mesures issues d'anciens boots afin de ne pas transformer une incohérence de
journal en vérité matérielle.

## Configuration de référence

| Composant | Référence actuelle |
|---|---|
| Machine | TRIGKEY Speed S5 |
| CPU | **AMD Ryzen 7 5700U** |
| CPU logiques | 16 |
| Stockage | NVMe interne |
| Ethernet | Realtek RTL8111/8168 (`10ec:8168`) |
| Boot | UEFI |
| Affichage de base | GOP UEFI |

## Incohérence à revalider au prochain boot

Un ancien relevé physique conservé dans le dépôt a publié :

```text
AMD Ryzen 7 5800H with Radeon Graphics
```

avec des identifiants PCI associés à une plateforme Cezanne.

Cette valeur entre en contradiction avec la configuration de référence
**Ryzen 7 5700U**. Elle ne doit donc plus être répétée sans réserve dans la
documentation courante.

Au prochain test physique, capturer explicitement dans le même manifeste :

```text
CPU brand string
CPUID family/model/stepping
RAM utilisable après ExitBootServices
PCI vendor/device/class de chaque contrôleur
résolution GOP
identité NVMe
identité Ethernet
identité xHCI
identité audio
identité GPU
```

Ce nouveau relevé deviendra la source de vérité matérielle.

## Ce qui a déjà motivé le bring-up TRIGKEY

La machine physique a imposé des travaux qui n'existent pas dans un simple boot
QEMU :

- topologie PCIe réelle ;
- RTL8111/8168 ;
- NVMe ;
- plusieurs contrôleurs xHCI ;
- HID physique ;
- 16 CPU logiques ;
- diagnostic sans dépendre uniquement d'une console série ;
- blackbox persistante ;
- télémétrie réseau ;
- BRDP ;
- image UEFI reproductible ;
- manifeste et noyau non strippé pour symbolisation.

## Affichage

Le chemin physique courant repose sur le **framebuffer GOP initialisé par
l'UEFI**.

Ce qui n'est pas revendiqué :

- pilote AMD natif ;
- accélération GPU ;
- changement dynamique de mode ;
- gestion multi-écran qualifiée.

Le chantier virtio-gpu de QEMU est distinct.

## Stockage

La TRIGKEY utilise un **NVMe**. Il doit être testé indépendamment des bancs ATA
de QEMU.

Les propriétés recherchées physiquement sont :

- détection fiable ;
- lecture ;
- écriture ;
- flush/durabilité ;
- erreurs/timeouts ;
- reboot ;
- persistance du profil navigateur.

## USB / xHCI / HID

Le matériel possède une chaîne USB réelle qui ne se comporte pas nécessairement
comme le xHCI virtuel.

Les tests physiques doivent qualifier :

- contrôleurs ;
- ports ;
- clavier ;
- souris ;
- hubs/récepteurs ;
- déconnexion/reconnexion ;
- stabilité sous charge.

## Ethernet

Le chemin physique de référence est **RTL8111/8168**.

La validation complète doit distinguer :

```text
PCI
→ BAR
→ IRQ
→ RX/TX
→ ARP
→ DHCP
→ DNS
→ TCP
→ TLS
→ navigateur
```

Une page qui ne s'affiche pas ne suffit jamais à accuser la couche Ethernet.

## Wi-Fi

Aucun support Wi-Fi complet n'est revendiqué dans la documentation courante.

## Audio

Le chemin AC'97 utilisé sous QEMU ne correspond pas à la chaîne audio de la
TRIGKEY. Le support matériel HDA reste un chantier distinct.

## GPU

Le périphérique virtio-gpu de QEMU et l'iGPU AMD physique sont deux cibles
différentes.

`GET_DISPLAY_INFO` sous virtio ne constitue aucune preuve concernant le GPU
physique.

## Diagnostic physique

La machine doit rester débogable même lorsque l'interface ou le navigateur ne
fonctionnent pas.

Outils de référence :

- framebuffer de diagnostic ;
- blackbox ;
- télémétrie ;
- BRDP ;
- extraction de blackbox ;
- symbolisation par le kernel ELF exact ;
- manifeste de l'image.

## Image de référence

Builder :

```powershell
powershell -ExecutionPolicy Bypass -File .\tools\reference\IMAGE-TRIGKEY.ps1
```

Avant flash :

```powershell
.\tools\reference\run-trigkey-single-usb-qemu.ps1 -Accel tcg -SkipBuild
```

À conserver ensemble :

```text
image
SHA256
manifest
HEAD Bouchaud
SHA Ladybird
kernel ELF non strippé
```

## Ordre du prochain passage physique

1. boot UEFI ;
2. identification CPU/RAM/PCI pour résoudre définitivement l'incohérence 5700U/5800H ;
3. GOP natif ;
4. SMP ;
5. USB/HID ;
6. NVMe ;
7. RTL8168 ;
8. DHCP ;
9. DNS ;
10. TLS ;
11. desktop ;
12. Bouchaud Navigateur ;
13. Example Domain ;
14. Wikipedia peinte ;
15. scroll/clic ;
16. onglets/process swaps ;
17. workers/sandbox ;
18. persistance ;
19. reboot ;
20. endurance ;
21. récupération blackbox/télémétrie.

Le GPU matériel et l'audio HDA viennent ensuite et ne bloquent pas la première
validation générale du système.
