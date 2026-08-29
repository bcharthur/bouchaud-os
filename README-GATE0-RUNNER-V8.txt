BOUCHAUD OS — GATE 0 RUNNER V8
================================

Etat prouve avant V8
--------------------
Le kernel Gate0 a deja :
- handoff SMP 10/10 et 20 000 entrelacements forces ;
- validation statique complete OK ;
- cargo bootimage OK ;
- commit e5654d4 ;
- transport serie TCP commit 2ad9d39.

Le premier boot runtime V7 a reellement atteint :
1/8 SMP discover 4
2/8 SMP AP 3/3
3/8 SMP load 4 CPUs
4/8 BrowserHost
5/8 WebContent
6/8 M11 ready
7/8 M11 GUI handshake
8/8 M11 document loaded
puis 20 s de dwell et "BOOT 1 : PASS".

L'echec suivant venait uniquement du runner PowerShell :
    foreach ($pid in ...)

PowerShell possede une variable automatique $PID en lecture seule et les noms
sont insensibles a la casse. Le cleanup essayait donc d'ecrire $PID APRES le
PASS.

V8
--
Remplace $pid par $qemuProcessId.
Ajoute aussi un garde : si un QEMU residue du crash V7 est encore vivant,
le runner refuse de commencer au lieu de tuer un processus inconnu.

Utilisation
-----------
Extraire ce ZIP a la racine en ecrasant RUN-GATE0-FINAL.ps1.

Puis :
    .\FIX-GATE0-RUNNER-V8.ps1

S'il signale un QEMU residuel et que c'est bien la VM Gate0 precedente :
    Get-Process qemu-system-x86_64 | Stop-Process -Force

Puis :
    .\RUN-GATE0-FINAL.ps1 -SkipStatic

La validation repart sur 3 boots frais.
Aucun changement kernel n'est effectue par V8.
