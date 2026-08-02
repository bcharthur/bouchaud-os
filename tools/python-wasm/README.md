# python-wasm-builder

Produit `src/assets/python/rustpython.wasm`, l'interpréteur Python embarqué
par la commande shell `python` de Bouchaud OS (`src/lang/python.rs`).

Ce n'est **pas** compilé par le noyau : RustPython tire des centaines de
dépendances et prend plusieurs minutes à compiler, ce qui serait absurde à
chaque build du kernel. On compile une fois, hors-ligne, et on committe le
`.wasm` résultant — même logique que les polices dans `src/assets/fonts/`.

## Pourquoi ça marche sans compilateur C ni WASI SDK

RustPython est écrit en Rust pur. `wasm32-wasip1` a un `std` officiel dans
rustup : pas besoin d'emscripten ni de wasi-sdk, juste `cargo` + la target.

On utilise `Interpreter::builder().add_frozen_modules(rustpython_pylib::FROZEN_STDLIB)`
plutôt que `Interpreter::without_stdlib()` : ce dernier ne fournit pas le
module `encodings`, indispensable dès l'initialisation de la VM.

On n'utilise **pas** `rustpython-stdlib` (modules natifs `_socket`, `_json`,
`posix`...) : ce crate dépend de `nix`/`rustix`/`termios`/`socket2`, qui ne
compilent pas pour `wasm32-wasip1`. Sans lui, les scripts qui font
`import os`/`import socket` échoueront à l'exécution — c'est voulu : le
noyau n'expose de toute façon aucun répertoire WASI "préouvert"
(`fd_prestat_get` renvoie systématiquement `EBADF`, voir `src/wasm/mod.rs`).

## Rebuild

```sh
rustup target add wasm32-wasip1
cd tools/python-wasm
cargo build --release --target wasm32-wasip1
cp target/wasm32-wasip1/release/python-wasm-builder.wasm \
   ../../src/assets/python/rustpython.wasm
```

## Valider en dehors du noyau (sans QEMU)

`wasmi` (la même version que `Cargo.toml` du noyau) tourne aussi sur host
std : un petit binaire qui recâble les mêmes fonctions `wasi_snapshot_preview1`
que `src/wasm/mod.rs` (fd_write/fd_read/proc_exit/args/environ/prestat/...)
permet de tester un script réel en quelques secondes, sans passer par
`bootimage` + QEMU. Utile pour vérifier qu'un futur bump de RustPython
n'ajoute pas de nouvel import WASI non géré (`wasm-tools`/lecture manuelle
de la section `import` du binaire pour lister les symboles requis).

## Limitations connues

- Pas de `pip` : aucune installation de paquets, le binaire est figé au
  moment du build.
- Pas d'accès disque/réseau depuis les scripts (`os`, `socket`, `open()`
  sur un vrai chemin échoueront).
- ~14 Mo : c'est le prix d'un vrai interpréteur Python (stdlib figée +
  moteur), à comparer aux ~700 Ko d'une police TTF déjà embarquée.
