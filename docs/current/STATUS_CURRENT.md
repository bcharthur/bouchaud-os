# État courant — relevé GitHub du 2026-11-02

```text
BOUCHAUD_CURRENT_HEAD=ba4244fea53434de84ce532bf9f6d7ae5a95342e
DATE=2026-11-02
MAIN_PROTECTED=false
OPEN_PRS=0
ACTIVE_BRANCHES=6
LAST_FULL_GREEN_SHA=UNKNOWN
CURRENT_BLOCKING_RED=UNKNOWN (aucun échec confirmé sur le HEAD au relevé)
```

**Ce HEAD est le dernier commit GitHub observé AVANT création de la branche de préparation.** CI Fast `41534263901` et Reliability V3 `41534263911` y sont désormais `success`; cela ne constitue toujours pas une qualification globale P13/Nightly/PHYSICAL. En présence d'une nouvelle poussée, ce tableau est obsolète et doit être régénéré par inspection des Actions.

- Branche par défaut : `main`. Commit du HEAD : `chore(ci): remove completed one-shot cleanup workflow after verified branch prune`; parent `ae24afad871ff78e6e260dd0f18ff1d36e255c51`; date du commit : 2026-11-02T22:05:55Z.
- **Protection de main : NON**, information signalée P0 de processus. Ne pas travailler directement sur main.
- Branches observées : `main`, `chatgpt/active`, `fix/nightly-ladybird-artifact-20261101`, `fix/proc-processus-snapshot-kvm-20261101`, et deux `backup/main-before-*`. Les deux PR de fix sont **fermées** lors de ce relevé ; leurs branches ne sont pas pour autant intégrées.
- PR ouvertes : 0 au relevé initial (la PR de cette préparation s'ajoutera).
- C22 : **CLOSED_AT_KVM** pour son contrat candidat 6/6 seulement. Le run PR `41518023294` référence le head source `5c3cd43a7a2a1c258388f9a81967abecdce51068`, mais le job a exécuté le checkout/merge SHA synthétique `84105df8bf1ca1cf31cb8da6eaab50e43a96383d` ; mémoire 6/6, croissance M2−M1 de 144 à 164 Kio sous le plafond 1024 Kio. Ce n'est donc pas une preuve « main same-SHA » au sens strict. Deux campagnes locales 6/6 supplémentaires rapportées par le mainteneur. Pas de validation PHYSICAL.
- P13 : **PARTIAL** ; primitives et sous-composants historiquement verts, mais absence de `P13_FINAL_STRICT_OK` sur le HEAD actuel avec KVM4 dédié, KVM8 dédié, SMP8 30 minutes, Nightly et SHA unique. Ne pas transposer des verts sur un autre commit.
- Nightly Full : dernier échec connu sur ancien `main` (`41424125876`), et run vert sur ancien candidat V26 (`41475434027`) — **aucun Nightly vert prouvé sur le HEAD actuel**.
- Livraison : la documentation et les workflows de la branche de préparation ne sont pas fusionnés sur `main`.

La propriété **LAST_FULL_GREEN_SHA=UNKNOWN** est volontaire : CI Fast/Integration/Reliability verts sur `b8436617` (runs 41532849519 / 41532849374 / 41532849530), mais cela n'est pas une qualification globale P13/Nightly/physique.

Lire [CI_AND_PROOFS.md](CI_AND_PROOFS.md) pour les preuves et limites.