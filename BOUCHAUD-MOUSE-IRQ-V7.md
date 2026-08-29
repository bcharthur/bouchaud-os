# Bouchaud OS — Mouse IRQ / bottom-half V7

## Pourquoi

Le V6 a reproduit le freeze sans Ladybird tout en montrant, juste avant l'arrêt,
un BSP vivant : `bsp_safe=1`, IF actif, PIT actif et reprise BKL courte.

Le premier mouvement de souris est alors devenu le déclencheur reproductible.
Le code précédent faisait ceci dans IRQ12 :

```text
IRQ12
 -> BKL::enter()
 -> mouse::handle_byte()
 -> signale_interface(Souris)
 -> WaitQueue::wake_all()
 -> BKL / parcours des tâches
 -> EOI
```

V7 interdit désormais cette architecture pour la souris.

## Architecture V7

```text
IRQ12 hard IRQ
  -> lire 8042
  -> décoder le paquet
  -> publier x/y/boutons/molette atomiquement
  -> signale_interface_irq()   # atomiques seulement
  -> EOI
  -> IRET

PIT, tick suivant
  -> smp_lock::try_enter()
  -> si succès:
       flush_interface_irq_bkl_held()
       -> wake_all_bkl_held()
       -> réveil du desktop/compositeur
  -> si échec:
       pending reste posé pour le tick suivant
```

Le hard IRQ souris :
- ne spin plus sur le BKL ;
- ne parcourt plus la table des tâches ;
- ne réveille plus directement une WaitQueue ;
- ne fait aucune allocation ni sortie série.

Le délai nominal du bottom-half est <= 1 tick PIT lorsque le BKL est disponible.

## Fragmentation souris

```text
src/drivers/input/
├── ps2_mouse.rs
└── mouse/
    ├── etat.rs
    ├── ps2.rs
    ├── paquet.rs
    └── diagnostic.rs
```

`ps2_mouse.rs` reste le module public historique via `drivers::mouse`.

## Fichiers remplacés

- `src/arch/x86_64/idt.rs`
- `src/drivers/input/ps2_mouse.rs`
- `src/kernel/sync/reveil.rs`
- `src/kernel/sync/wait_queue.rs`
- `src/arch/x86_64/cpu/idle/trace.rs` (extension du V6)

Le patch est **incrémental sur V6/V5.1**. Il ne remplace pas les autres fichiers
BKL, `persistance.rs`, `gui/client.rs` ou le Performance Observatory.

## Nouveaux logs

Toutes les secondes, le rapport existant affiche maintenant :

```text
[MOUSE-IRQ]
 phase=...
 entries=...
 bytes=...
 eoi=...
 exit=...
 packets=...
 changed=...
 deferred=...
 irq_signals=...
 irq_flushes=...
 irq_woken=...
 pending=...
```

Interprétation essentielle :

- `entries > eoi` : IRQ bloquée avant ACK ;
- `eoi == entries` mais `exit < entries` : problème après EOI ;
- `signals` monte mais `flushes` reste fixe et `pending=1` :
  PIT/BKL bottom-half ne progresse plus ;
- `flushes` monte et la souris reste vivante : déport IRQ validé.

## Test

Après extraction à la racine du dépôt :

```powershell
git status --short
git diff --check

.\run.ps1 -Ladybird -LadybirdUrl "https://www.google.com/" |
    Tee-Object -FilePath mouse-v7-desktop.log
```

Premier test : ne pas ouvrir Ladybird. Attendre le bureau puis bouger la souris
pendant 30 à 60 secondes, ouvrir le terminal, déplacer la fenêtre et cliquer.

Puis :

```powershell
python .\tools\perf\analyse-mouse-v7.py .\mouse-v7-desktop.log
```

Si le bureau reste vivant, refaire ensuite Google.

## Important

Le clavier conserve encore son chemin BKL historique lorsque l'octet reçu est
réellement un octet clavier. En revanche, un octet auxiliaire souris qui arrive
sur IRQ1 est redirigé vers le nouveau chemin BKL-free.

On ne généralisera la même architecture à IRQ1 qu'après validation de V7.
