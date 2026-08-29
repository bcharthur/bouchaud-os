BOUCHAUD OS — GATE 0 FINAL V6
================================

1) Extraire TOUT le ZIP a la racine du depot.
2) Lancer :
       .\APPLY-GATE0-FINAL.ps1
3) Si le script affiche "GATE 0 STATIC = VERT", lancer :
       .\RUN-GATE0-FINAL.ps1

Ce package part exactement de :
9734e8af6e5d223a0554b29c5538d0d265834cbf

Correction centrale :
- suppression de ctx.rsp=0 comme pseudo-verrou SMP ;
- outgoing reste on_cpu>=0 + switching_out=true ;
- SWITCH_PENDING par CPU ;
- aucune publication/runqueue avant le changement physique de pile ;
- complete_switch_handoff() depuis la continuation entrante ;
- wake concurrent conserve sans publication precoce ;
- zombie recyclable uniquement apres abandon de pile ;
- une seconde commutation ne peut pas ecraser un handoff pending.

Le runner ajoute un mode -Gate0Autostart qui ne change PAS le comportement
normal de run.ps1. Il autostart Ladybird M11 uniquement pour cette validation.

Gate 0 n'est declare TERMINE qu'apres 3/3 boots SMP4, chacun avec :
- SMP4_DISCOVERED count=4
- SMP4_AP_STARTED count=3 expected=3
- SMP4_SCHEDULER online=4
- BROWSER_HOST_INITIALIZED
- WEBCONTENT_READY
- M11_READY
- M11_GUI_HANDSHAKE_OK
- M11_DOCUMENT_LOADED
- puis 20 secondes stables sans panic/double fault/BKL violation.

Aucun git clean, reset --hard ou suppression de fichiers locaux n'est effectue.

V3 ajoute des assertions de residence actives en release et durcit le stress concurrent.
V4 interdit aussi le recyclage/comptage d'une tache tant que son handoff est pending.

V5 corrige le faux positif du controle final : le site diagnostic 54 etait encore documente sous l'ancien nom `complete_retired` sans parentheses. Le controle strict est conserve ; la legende est maintenant `complete_switch_handoff`.

V6 :
- reprend sans reset un APPLY V5 interrompu avec uniquement les 3 fichiers Gate0 modifies ;
- remplace le stress Barrier/atomiques qui pouvait se bloquer par un handshake concurrent borne ;
- chaque run force 2000 fois la fenetre save-rsp -> stack-left ;
- 10 runs = 20 000 entrelacements verifies avant cargo/bootimage/commit.
