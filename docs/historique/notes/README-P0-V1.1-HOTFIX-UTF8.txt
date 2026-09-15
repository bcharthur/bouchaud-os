BOUCHAUD OS P0 #1 - v1.1 HOTFIX UTF-8
=======================================

CAUSE DU BUILD CASSE
--------------------
Le patch v1 lisait les fichiers avec:

  Get-Content -Raw

Sous Windows PowerShell 5.1, un fichier UTF-8 sans BOM peut etre decode comme
ANSI. Les octets UTF-8 de "nœuds" ont alors ete interpretes comme "nÅ“uds",
puis re-encodes en UTF-8. Rust rencontre ensuite le caractere U+201C dans un
identifiant et refuse de parser le fichier.

Le patch scheduler lui-meme n'est pas en cause.

POUR REPARER
------------
Depuis la racine du repo:

  .\REPAIR-P0-SCHED-IPI-TARGETED.ps1

Le script:
1. trouve le dernier backup cree AVANT le patch v1;
2. restaure byte-for-byte thread.rs et idt.rs;
3. verifie que "nœuds" est de nouveau correct;
4. reapplique le P0 avec un decodeur UTF-8 strict;
5. lance cargo check.

Puis:

  .\VERIFY-P0-SCHED-IPI-TARGETED.ps1 -Build
  .\run.ps1

Le fichier APPLY-P0-SCHED-IPI-TARGETED.ps1 du ZIP est aussi remplace par une
version v1.1 UTF-8-safe pour les prochaines applications.

Les warnings Git LF/CRLF ne sont pas la cause de l'echec.
