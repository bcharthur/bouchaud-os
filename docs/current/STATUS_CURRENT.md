# État courant

## Workflows

| Workflow | Résultat |
|---|---|
| CI Fast | SUCCESS |
| Reliability V3 | SUCCESS |
| Integration | SUCCESS |
| os-primitives | SUCCESS |
| ladybird-native-browser | FAILURE de convergence |

## Jobs navigateur

Build, smoke BrowserHost, cache/SQL, WPT, sites réels, robustesse,
crash-services, ordre Worker, piège Compositor, performance, endurance TCG
10 min et endurance KVM 20 min sont verts.

Restent rouges :
- mémoire après onglets fermés, diagnostic ;
- endurance KVM A/B ;
- convergence finale.

## P2/P9 mémoire

Avant :
```text
contexts_live       3 → 13 → 22
backing_store KiB   4623 → 50857 → 97092
```

Après discard direct :
```text
Compositor RSS      41652 → 48276 → 48280 KiB
contexts_live       2 → 3 → 3
backing_stores      4 → 2 → 4
backing_store KiB   16822 → 4623 → 16822
```

La fuite linéaire précédente a disparu. Le banc reste rouge parce que seuls
17/20 process swaps ont été observés et parce que l'analyseur considère encore
`2 → 3 → 3` comme croissance.

## Compositor

Le vieux trap `DrawGlyphRun`/font manquante n'est plus reproduit par smoke,
test ciblé, robustesse et endurance longue.

## KVM A/B

Les deux bras produisent 58 cycles et 58 frames en ~304 s, sans frame >30 s.
Le premier bras échoue uniquement car le banc compte zéro Compositor malgré
l'absence de mort, faute, assertion et abandon de lien. Le second bras compte
correctement un Compositor.
