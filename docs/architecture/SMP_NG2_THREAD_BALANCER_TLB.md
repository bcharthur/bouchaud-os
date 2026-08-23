# SMP-NG2 — thread load balancer + TLB shootdown

## Objectif

NG2 retire la contrainte `processus = un seul CPU` pour le userland. Le BKL
reste présent pour sérialiser le noyau historique, mais plusieurs threads d'un
même processus peuvent maintenant exécuter du ring3 simultanément sur plusieurs
CPU logiques.

## Scheduler

Chaque `Task` user possède :

- `affinity_mask` : CPU autorisés ;
- `runq_cpu` : propriétaire logique de la runqueue Ready ;
- `last_cpu` : dernier CPU exécuté ;
- `on_cpu` : CPU actuellement running, `-1` sinon.

Les fils noyau restent CPU0. Les nouveaux threads sont placés selon un score qui
combine pression Ready, tâches running, charge mesurée et une petite pénalité
CPU0. Un CPU sans travail local vole une tâche Ready d'une autre runqueue.

## TLB shootdown

Le même `AddressSpace` peut être actif sur plusieurs CPU. Chaque espace maintient
`active_cpus`, et les mutations dangereuses suivent cet ordre :

1. modifier les PTE ;
2. invalider localement ;
3. publier PML4/plage ;
4. envoyer l'IPI `0xF2` ;
5. le CPU dont CR3 correspond invalide par INVLPG ou recharge CR3 ;
6. ACK atomique ;
7. l'émetteur attend tous les ACK ;
8. `unmap` libère ensuite seulement les frames.

Le handler TLB ne prend jamais le BKL.

Le premier jalon broadcast à tous les CPU online puis filtre par CR3. C'est
volontairement plus robuste pendant la migration. `active_cpus` est déjà tenu à
jour pour permettre un shootdown ciblé plus tard.

## Page faults concurrents

Deux threads du même `AddressSpace` peuvent fauter simultanément sur la même
page. Si le second attend le BKL et découvre que le premier a déjà matérialisé
la page, un fault *not-present* est considéré résolu. Une vraie faute de
protection reste fatale.

## Observabilité

Topbar :

```text
CPU: 52% [98/71/22/17]
```

Journal :

```text
[SMP-LOAD] total=... tlb=... c0=... rq=... cur=pid:tid steal=... mig=...
```

Création de tâches :

```text
[SMP-TASK] ... rq=2 last=1 aff=0xf ...
```

## Limites volontaires de NG2

- le BKL n'est pas retiré ;
- `Rc<RefCell<Process>>` existe encore, ses accès noyau restant sérialisés par le
  BKL ;
- les runqueues sont représentées par ownership (`runq_cpu`) dans la table
  globale tant que le scheduler est sous BKL ;
- le réseau reste partiellement polling/backoff ;
- pas encore d'accélération GPU.

La priorité de NG2 est le gain qui manque au navigateur : exploiter réellement
plusieurs CPU pour les pthreads Ladybird tout en gardant le MM correct.
