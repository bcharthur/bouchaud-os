# Bouchaud OS — Native Core Game-Changer V11B

## Intention

V11B crée les frontières qui permettront de fragmenter les gros monolithes
restants sans transformer Bouchaud OS en clone de Linux.

La règle est explicite :

```text
compat/linux
    ↓ traduction ABI
kernel/native
    ↓ concepts Bouchaud
scheduler / sync / objects / memory / network
```

Le cœur ne dépend pas de constantes ou d'objets Linux. Linux n'est qu'un
adaptateur de compatibilité périphérique.

## 1. WaitSource natif

Nouveau sous-système :

```text
src/kernel/sync/
├── wait_source.rs
└── wait_source/
    ├── etat.rs
    ├── ticket.rs
    ├── attente.rs
    ├── signal.rs
    └── diagnostic.rs
```

Il centralise le contrat :
- génération ;
- ticket ;
- recheck avant parking ;
- attente avec deadline ;
- signal normal ;
- publication hard-IRQ différée ;
- flush bottom-half avec BKL déjà détenu ;
- statistiques.

`kernel::sync::reveil` est migré dessus : ce n'est pas un simple squelette.

## 2. ReadinessSource par objet

```text
src/kernel/object/
├── readiness.rs
└── readiness/
    ├── etat.rs
    ├── ticket.rs
    ├── wait.rs
    ├── signal.rs
    └── diagnostic.rs
```

C'est la fondation pour remplacer progressivement la readiness globale de
poll/ppoll par des générations PAR OBJET.

Règle de correction :
1. ticket par source ;
2. lecture readiness ;
3. registration ;
4. recheck ;
5. parking ;
6. recheck après réveil.

On ne fait donc PAS le dangereux `wake_all global -> wake_one global`.

## 3. Frontière kernel/native

```text
src/kernel/native/
├── mod.rs
├── scheduler.rs
├── sync.rs
├── io.rs
├── memory.rs
├── network.rs
└── time.rs
```

Ces interfaces sont propres à Bouchaud OS. Elles donnent une destination stable
aux futures migrations de `compat/linux/mod.rs`.

## Après V11B

Les prochains game changers deviennent plus faciles :

- V11C : `thread.rs` -> table/current/switch/blocking/futex/lifecycle/accounting ;
- V11D : futex natif à buckets/verrou local ;
- V11E : migration poll/readiness vers `ReadinessSource` ;
- V11F : réseau RX/TX/wait/readiness ;
- V11G : `compat/linux` réduit à une pure traduction ABI.

Aucun noyau Linux, composant Windows ou composant macOS n'est intégré.
