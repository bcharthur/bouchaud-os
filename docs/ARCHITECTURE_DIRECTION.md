# Architecture — état actuel et direction

Ce document complète `ARCHITECTURE.md`. Aucune cible n'est présentée comme livrée.

## CURRENT — ce qui existe

```text
applications/services ring 3 (ELF, ports Ladybird)
                    |
          GUI protocol / syscalls
                    |
     WM + compositeur noyau (ring 0)
                    |
       backbuffer logiciel / framebuffer
                    |
             QEMU x86_64
```

Le noyau contient aujourd'hui fenêtres, focus, composition, damage, widgets du
shell et routage d'entrée. Les clients ring 3 emploient protocole GUI et surfaces
partagées, mais le window server n'est pas un service ring 3. Le rendu est
logiciel ; aucun GPU Bouchaud n'est revendiqué.

Processus, VM, scheduler SMP, VFS, signaux, réseau et ABI Linux-compatible sont
également dans le chemin actuel. Cette ABI permet les ports : elle est une
**couche de compatibilité**, pas la définition permanente de Bouchaud.

## TARGET — direction non implémentée

```text
applications ring 3 natives ou portées
                    |
      APIs Bouchaud / IPC / shared surfaces
                    |
      window server + compositeur ring 3
                    |
       petite interface graphics/kernel
                    |
         framebuffer ou backend GPU
```

Migration incrémentale et mesurée : Bouchaud peut rester hybride/pragmatique,
sans microkernel académique pur. Une politique ne sort que lorsque primitive,
protocole, reprise et tests préservent le chemin x86_64.

Probablement userland à terme : WM/compositeur, shell, Git, moteurs applicatifs
et services de haut niveau. Restent noyau ou ouverts : scheduler, VM, primitives
IPC/shared memory, drivers essentiels et primitives sécurité/isolation.

## ABI Linux et primitives natives

```text
applications portées -> ABI Linux de compatibilité -> primitives actuelles
applications futures -> API native Bouchaud -> IPC / objets / mémoire / surfaces
```

La première pile existe ; la seconde est une direction. Des handles,
shared-memory, protocole GUI et surfaces existent déjà, sans constituer une API
native stabilisée. Handles typés, IPC, capabilities et surfaces versionnées sont
des candidats planifiés jusqu'à spécification/tests de compatibilité.

## Stratégie matériel

**0.1 : QEMU x86_64 normatif.** Une seule machine physique choisie peut être NICE
TO HAVE. « x86_64 » ne signifie pas tous les laptops.

Pour un futur **Bouchaud One**, choisir d'abord le matériel. Le support universel
exigerait ACPI, NVMe/AHCI, xHCI, USB HID, Wi-Fi, HDA, batterie, suspend/resume et
GPU ; ce n'est pas une exigence 0.1.

## AArch64 et plateformes futures

AArch64/Raspberry Pi restent dans la vision, hors 0.1 :

```text
arch/x86_64          arch/aarch64
      |                    |
platform/pc   platform/qemu_virt   platform/raspberry_pi
```

L'ISA fournit les mécanismes CPU sans connaître la carte ; la plateforme décrit
interruptions/timers/périphériques. Les répertoires présents sont une fondation,
pas une preuve de boot AArch64 ou Raspberry Pi.

## Ordre proposé

1. stabiliser/mesurer QEMU x86_64 0.1 ;
2. versionner protocole GUI, surfaces et erreurs ;
3. définir les primitives minimales IPC/shared-memory/isolation ;
4. déplacer une politique à la fois derrière les mêmes tests ;
5. introduire un backend graphique sans GPU obligatoire ;
6. valider séparément QEMU `virt` AArch64, puis un Raspberry Pi choisi.
