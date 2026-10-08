# Bouchaud OS — contexte de référence pour humain ou IA

**Branche :** `claude/ladybird-observability-performance`  
**HEAD :** `9e086175c8e77c3a48d11eee8d2aa86442c78d67`  
**Ladybird upstream :** `cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6`  
**Référence physique :** TRIGKEY Speed S5  
**Date :** 2026-10-08

Ce fichier est le point d'entrée recommandé.

## Hiérarchie de vérité

1. code du HEAD courant ;
2. logs et CI du même HEAD ;
3. `docs/current/STATUS_CURRENT.md` ;
4. `docs/current/P1_P13_CURRENT.md` ;
5. documentation détaillée existante ;
6. notes historiques.

Les anciennes documentations de plateforme non utilisées ne sont pas des
sources autoritatives de la trajectoire actuelle.

## État global

```text
CI Fast                     SUCCESS
Reliability V3              SUCCESS
Integration                 SUCCESS
os-primitives               SUCCESS

Ladybird build              SUCCESS
BrowserHost smoke           SUCCESS
cache + SQL                 SUCCESS
WPT                         SUCCESS
sites réels                 SUCCESS
robustesse                  SUCCESS
crash services              SUCCESS
ordre Worker                SUCCESS
piège Compositor            SUCCESS
performance                 SUCCESS
endurance TCG 10 min        SUCCESS
endurance KVM 20 min        SUCCESS

mémoire onglets             FAILURE diagnostic
endurance KVM A/B           FAILURE
convergence P13             FAILURE
```

## Matériel

La référence physique actuelle est la **TRIGKEY Speed S5 mesurée par Bouchaud
OS**.

## Navigateur

Le produit peut être appelé **Bouchaud Navigateur**. Son moteur dérive de
Ladybird. Les notices et licences du moteur restent explicites.

## Lire ensuite

1. `docs/current/STATUS_CURRENT.md`
2. `docs/current/P1_P13_CURRENT.md`
3. `docs/current/ARCHITECTURE_CURRENT.md`
4. `docs/current/TRIGKEY_REFERENCE.md`
5. `docs/current/BOUCHAUD_NAVIGATEUR.md`
6. `docs/current/CI_AND_PROOFS.md`
7. `docs/current/HISTORY_AND_DECISIONS.md`
8. `docs/current/ROADMAP_CURRENT.md`
9. `docs/current/KNOWN_LIMITATIONS.md`

## Règles de reprise

- pas de reset/clean destructif ;
- préférer un worktree séparé ;
- toujours relever branche, HEAD et état Git ;
- STATIC ≠ HOST ≠ QEMU-TCG ≠ QEMU-KVM ≠ PHYSICAL ;
- pas de faux vert ;
- pas de relâchement de budget sans preuve ;
- pas de confusion entre périphérique virtuel et matériel physique.
