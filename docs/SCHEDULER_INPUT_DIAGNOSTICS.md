# Diagnostic ordonnanceur / clavier — WM + navigateur

Base du correctif : `main` apres PR #154 (`bd55d1f`).

## Corrections

- demasquage explicite IRQ0/IRQ1 cote PIC ;
- activation du scanning clavier (`F4`) et traduction Set-1 du 8042 ;
- `try_scancode()` preserve IF ;
- decode E0 strictement non bloquant ;
- WM en priorite `Interactive` ;
- preemption differee quand IRQ0 interrompt un syscall ;
- suppression du double HLT de `sleep_ticks` ;
- watchdog WM depuis IRQ0 ;
- diagnostic scheduler + clavier toutes les ~5 s ;
- routage defensif 8042 clavier/souris via le bit AUX.

## Lignes attendues

    [sched] switches=... irq-preempt=... deferred=... wm-age=... ms ready=X/Y
    [kbd] irq=... attente=... perdus=... last=0x.. status=0x.. cfg=0x.. PIC1=0x.. ACK(F6/F4)=0xfa/0xfa

Si IRQ0 vit mais que le WM ne revient plus pendant >= 2 s :

    [sched-watchdog] desktop sans heartbeat depuis ... ms ; ...

## Interpretation

- `ACK(F6/F4)=0xfa/0xfa` : clavier present et scanning active.
- bit 1 de `PIC1` a 0 : IRQ1 demasquee.
- `irq=0` pendant la frappe : panne IRQ/8042/PIC.
- `irq` monte mais `attente` monte : WM ne consomme plus.
- `wm-age` proche de 0 : WM itere.
- watchdog sans `[sched]` : timer vivant, WM affame/bloque.
- `deferred` monte : les ticks tombent souvent dans les syscalls ; le chemin
  de preemption differee est necessaire.

## Test

1. Bureau seul : taper dans le terminal, attendre 10 s.
2. Verifier `[kbd]` et `[sched]`.
3. Ouvrir Bouchaud Browser.
4. Laisser tourner 20–30 s.
5. Verifier que `[ps]`, `[sched]`, `[gui]` continuent et qu'aucun watchdog
   persistant n'apparait.


## Correctif v3 — coexistence clavier/souris sur le 8042

Le boot de validation a montre que le clavier fonctionnait avant `desktop`, puis
que son compteur IRQ restait fige apres `mouse::init()`. La souris et le clavier
partagent le 8042 : leurs transactions d'initialisation sont maintenant faites
interruptions coupees, le command byte conserve explicitement les deux ports, et
le scanning clavier (`F4`) est rearme apres la negociation IntelliMouse.
