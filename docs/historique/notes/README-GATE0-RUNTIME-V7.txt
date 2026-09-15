BOUCHAUD OS — GATE 0 RUNTIME V7
================================

Pourquoi V7 existe
------------------
Le kernel/handoff V6 a passe :
- stress handoff 10/10, soit 20 000 entrelacements forces ;
- clavier 8/8 ;
- damage 26/26 ;
- GUI 9/9 ;
- BKL 5/5 ;
- commutation SMP 9/9 ;
- PE 28/28 ;
- cargo check ;
- cargo bootimage ;
- commit e5654d4.

Le premier RUN V6 a affiche 0/8 pendant 360 s.
Ce verdict n'etait pas exploitable : V6 essayait de relire en direct des
fichiers que Start-Process utilisait comme RedirectStandardOutput/Err.
Sur Windows cette methode n'est pas un canal de streaming fiable pour notre
preuve serie.

V7 n'observe plus stdout de PowerShell.

Architecture V7
---------------
run.ps1 recoit :
    -Gate0SerialPort <port>

Il configure QEMU :
    -serial tcp:127.0.0.1:<port>,server=on,wait=on,nodelay=on

QEMU ATTEND le collecteur avant de booter. Donc :
- aucun octet de boot n'est perdu ;
- aucun verrou de fichier de log n'entre dans la boucle d'observation ;
- le runner lit COM1 directement par TCP ;
- la sortie host run.ps1 est conservee uniquement pour diagnostic.

Correction d'un second faux critere
-----------------------------------
V6 exigeait :
    SMP4_SCHEDULER online=4

Cette chaine n'est pas un marqueur serie du noyau actuel.

V7 exige a la place :
- SMP4_DISCOVERED count=4
- SMP4_AP_STARTED count=3 expected=3
- une vraie ligne [SMP-LOAD] avec c0,c1,c2,c3
- BROWSER_HOST_INITIALIZED
- WEBCONTENT_READY
- M11_READY
- M11_GUI_HANDSHAKE_OK
- M11_DOCUMENT_LOADED
- puis 20 secondes sans fatalite.

Installation sur l'etat actuel
------------------------------
Extraire le ZIP a la racine du depot puis :

    .\UPGRADE-GATE0-RUNTIME-V7.ps1

Ce script ajoute uniquement le transport serie TCP a run.ps1 et le commit.

Puis :

    .\RUN-GATE0-FINAL.ps1 -SkipStatic

`-SkipStatic` est adapte maintenant parce que e5654d4 vient deja de passer
l'integralite de la validation statique + bootimage. Pour une revalidation
complete, omettre -SkipStatic.

Verdict final
-------------
Le runner n'imprime `GATE 0 = TERMINE` qu'apres 3/3 boots reels.
