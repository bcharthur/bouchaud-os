# Limites produit — relevé 2026-11-03

Chaque statut est relatif aux preuves disponibles, pas à une impossibilité architecturale.

| Priorité | Limite quotidienne | Statut |
|---|---|---|
| P0 | Baseline P13 stricte KVM4/KVM8 + endurance 30 min + SHA unique | **NON QUALIFIÉ — runners dédiés absents** |
| P0 | Protection de `main` (PR + checks sans contournement) | **ABSENTE — ruleset prêt (`tools/github/ruleset-main.json`), à appliquer** |
| P0 | Validation PHYSICAL Trigkey du HEAD courant : boot, réseau, stockage, récupération | **UNKNOWN / À REFAIRE** |
| P0 | Persistance sûre : read/write/mmap, fsync, panne d'alimentation et recovery | **NON DEMONTRÉE** |
| P1 | Install/update/rollback reproductibles et signature des artefacts | **A/B prouvé sur hôte, non branché au chargeur ; signature à faire** |
| P1 | Isolation et supervision des services userland ; pilotes avec privilèges minimaux | **PARTIEL : 1er service natif supervisé (healthd/serviced) qualifié TCG ; reste des services encore noyau** |
| P1 | Latence IPC native SMP1 (p50 ≈ 4 ms = un quantum UP) | **MESURÉE, NON ATTRIBUÉE** |
| P1 | HDA, alimentation/batterie, suspend/resume, thermique | **UNKNOWN** |
| P1 | GPU physique/3D, multi-écran et stabilité du compositor | **UNKNOWN PHYSICAL** |
| P1 | Wi-Fi, Bluetooth, périphériques USB réels et touchpad | **UNKNOWN PHYSICAL** |
| P2 | Package management, pilotes supplémentaires et ergonomie desktop | **NON QUALIFIÉ** |
| P3 | IA locale, Activity OS, Dioxus, VPN/Tor, Compute Fabric | **FUTUR : NE PAS DÉVELOPPER DANS LE CLOSEOUT** |

Ce tableau n'invalide pas les implémentations existantes. Il interdit de les présenter comme daily-driver sans protocole mesuré.
