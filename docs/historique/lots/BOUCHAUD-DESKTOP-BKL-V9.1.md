# Bouchaud OS — Desktop BKL Scoped V9.1

## Correctif de compilation

V9 échouait avec `E0753 expected outer doc comment` dans :

- `src/gui/framebuffer.rs`
- `src/gui/reveil.rs`

Cause : les wrappers V9 utilisaient `include!()` pour injecter ces fichiers
dans un bloc `mod legacy { ... }`. Les deux fichiers historiques commencent par
des commentaires de documentation internes `//!`. Rust refuse ce type de
documentation lorsqu'elle provient d'une expansion `include!`.

V9.1 ne modifie aucune logique BKL, GUI ou scheduler.

Le correctif remplace simplement :

```rust
mod legacy {
    include!("framebuffer.rs");
}
```

par :

```rust
#[path = "framebuffer.rs"]
mod legacy;
```

et applique la même correction à `reveil.rs`.

Les fichiers historiques deviennent donc de vrais modules Rust ; leurs `//!`
redeviennent légaux.

## Fichiers remplacés

```text
src/gui/framebuffer_v9.rs
src/gui/reveil_v9.rs
```

Aucun autre fichier V9 n'est remplacé.

## Test

Après extraction par-dessus V9 :

```powershell
git diff --check

.\run.ps1 -Ladybird -LadybirdUrl "https://www.google.com/" |
    Tee-Object -FilePath desktop-bkl-v9.1-google.log
```

Si le build passe, conserver le même protocole de test runtime V9 puis lancer :

```powershell
python .\tools\perf\analyse-kthread-bkl-v9.py .\desktop-bkl-v9.1-google.log
```
