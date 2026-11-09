# P10 GPU — report sur main (9 novembre 2026)

Base: `6bdcb36f3432d202d37b4562f43dee7363d0852f`; branche `feat/p10-mainline-gpu-20261109` ; aucune modification DOOM historique.

- Capset VIRGL et contexte 3D réellement négociés ; commandes fences et contrôle des réponses.
- Upload d'une surface cliente ring3 vers texture via DMA, BLIT VIRGL, un seul RESOURCE_FLUSH complet, retour CPU.
- Travailleur de présentation asynchrone avec snapshots réutilisables et statistiques de pertes.
- Branchement P10 dans `widgets::compose_client` et `window_manager` versionnés dans cette branche.
- Sélection de `Mode::Copie` spécifique au P10, **sans** toucher à `TAMPON_DIRECT_VOULU`. P10+.2 garde sa politique.
- Compilation optionnelle des dépendances Vulkan pour Ladybird ; **aucun GPU Skia Vulkan ring3 encore prouvé**.

Limites: seulement une fenêtre sans occlusion, QEMU VIRGL expérimental, ni pilote AMD physique ni performance démontrée. Ne pas fusionner sans CI et QEMU pixels/latence.
