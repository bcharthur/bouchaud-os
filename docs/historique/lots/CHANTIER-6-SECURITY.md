# Chantier 6 — Security Architecture V2.1

Base exacte : `178ec56` (`c7: validate native IPC from ring3`).

Cet overlay peut etre extrait directement **au-dessus du V1 non committe actuel** : les ancrages V1 sont idempotents et les nouveaux fichiers security sont remplaces par la V2.1. Aucun reset/stash n'est necessaire.

L'overlay installe une frontière de sécurité obligatoire devant **les deux ABI**
(Linux-compat et Bouchaud native), sans réintroduire le BKL dans le cœur de la
politique.

Implémenté dans cette vague :

- credentials par processus : UID/GID réel, effectif, sauvegardé + groupes;
- capacités explicites et transitions d'autorité monotones;
- aucune élévation par simple nom `BrowserHost` : l'identité borne les capacités;
- profils `System`, `User`, `BrowserBroker`, `BrowserContent`, `Untrusted`;
- moteur de chemin canonique avant décision de sandbox/device/exec;
- `openat` / `mkdirat` / `unlinkat` conservent réellement le `dirfd` côté politique **et** backend;
- un descripteur de fichier ordinaire ne peut plus servir de base `*at`;
- audit des refus de chemin avec cible canonique + raison stable;
- DAC Unix owner/group/other sur open, création, suppression et renommage;
- `/tmp` en `01777` avec règle sticky;
- propriétaire des nouveaux fichiers/répertoires/memfd = euid/egid du processus;
- W^X strict, même pour System;
- RX anonyme derrière la capacité `JIT`;
- `MAP_SHARED|PROT_WRITE` vérifie le mode du fichier **et** l'accès du FD;
- `mprotect(PROT_WRITE)` revalide les mappings partagés;
- exécution : bit `x`, rejet des exécutables world-writable/non fiables;
- restriction des devices privilégiés, raw sockets / AF_PACKET;
- contrôle `kill`, `tkill`, `tgkill` entre identités;
- vrais `setuid` / `setgid` à la place du no-op historique;
- `PR_SET_NO_NEW_PRIVS` / `PR_GET_NO_NEW_PRIVS`;
- IPC natif : contrôle du transfert de handles + plafond par objet SHM;
- audit borné des refus et marqueurs `[SECURITY-DENY]`;
- nettoyage du contexte sécurité dans `Process::drop`;
- tests de politique host + preuve ring3 SMP4 + workflow CI dédié.

Le plafond SHM est volontairement décrit comme **par objet** : cette vague ne
prétend pas fournir une comptabilité cumulative de mémoire native.

## Application

Depuis la racine du dépôt, HEAD doit encore être `178ec56` :

```powershell
Expand-Archive .\bouchaud-os-chantier-6-security-v2.1.zip -DestinationPath . -Force
.\APPLY-SECURITY.ps1
```

Puis :

```powershell
python tools/security/verifie-security.py
git diff --check
.\tools\security\run-host-tests.ps1
cargo check
cargo bootimage
.\tools\security\run-security-ring3.ps1
```

Ne commit pas avant `SECURITY_ARCH_V21_OK`, `SECURITY_HOST_OK`, `SECURITY_RING3_OK` et la regression native IPC verte.
