# python-wasm-builder

Produit `src/assets/python/rustpython.wasm` : l'interpréteur **Python complet**
des commandes shell `python` / `pip` de Bouchaud OS (`src/lang/python.rs`,
`src/lang/pip.rs`).

Ce n'est **pas** compilé par le noyau : RustPython tire des centaines de
dépendances et prend plusieurs minutes à compiler. On compile une fois,
hors-ligne, et on committe le `.wasm` (~16 Mo) — même logique que les polices
de `src/assets/fonts/`.

## Contenu du binaire

- **RustPython 0.5** (Python 3.13) : compilateur + VM, écrits en Rust pur —
  aucune chaîne C/emscripten requise, la cible `wasm32-wasip1` de rustup suffit.
- **stdlib Python figée** (`freeze-stdlib` + `rustpython-pylib`) : les modules
  purs Python de la stdlib sont dans le binaire.
- **stdlib native** (`rustpython-stdlib`, feature `host_env`) : `os`/`posix`,
  `_io` (le vrai `open()`), `json`, `re` (`_sre`), `math`, `hashlib`, `zlib`
  (via `zlib-rs`, Rust pur), `binascii`, `csv`, `random`, `struct`, `array`,
  `unicodedata`... Les modules impossibles en WASI (`socket`, `ssl`, `mmap`,
  `subprocess`...) sont exclus par `cfg` en amont.
- **Modes** du `main.rs` : REPL interactif (aucun argument), `-c "code"`,
  `-` (script sur stdin), `chemin.py [args]` (ouvert via WASI).

Côté noyau, chaque appel WASI (`path_open`, `fd_readdir`, `poll_oneoff`...)
est traduit vers le RAMFS et le clavier/VGA par `src/wasm/mod.rs` : les
scripts voient le même système de fichiers que le shell, avec les permissions
Unix de la session.

## Rebuild

Depuis un répertoire **hors du dépôt** (le `.cargo/config.toml` du dépôt
cible le noyau avec build-std, il ne faut pas en hériter) :

```sh
rustup target add wasm32-wasip1
cp -r <repo>/tools/python-wasm /tmp/pw && cd /tmp/pw
cargo build --release --target wasm32-wasip1
cp target/wasm32-wasip1/release/python-wasm-builder.wasm \
   <repo>/src/assets/python/rustpython.wasm
```

## Valider hors QEMU

Un harnais hôte (wasmi + les mêmes fonctions WASI que `src/wasm/mod.rs`,
adossé à un répertoire sandbox) permet de tester en secondes : fichiers,
`seek`/`tell`, `os.listdir`, REPL, `input()`, imports depuis un
`site-packages`, tracebacks, codes de sortie. En cas de bump de RustPython,
relister la section `import` du `.wasm` pour vérifier qu'aucun nouvel import
WASI n'apparaît sans implémentation noyau.

## Points d'attention connus

- **isatty / bufferisation** : le noyau retire les droits `fd_seek`/`fd_tell`
  aux fds 0-2 dans `fd_fdstat_get` — c'est ainsi que wasi-libc détecte un
  terminal et que la sortie Python est line-buffered. Le `main.rs` flush aussi
  `sys.stdout`/`sys.stderr` explicitement (le `proc_exit` est immédiat).
- **Couleurs** : la console VGA n'interprète pas l'ANSI ; le noyau passe
  `PYTHON_COLORS=0` / `NO_COLOR=1` / `TERM=dumb` (convention CPython 3.13).
- **Démarrage** : l'init de la VM Python (exécutée par wasmi) prend ~0,5 s en
  natif, ~30 s sous QEMU TCG. Le module wasm parsé est mis en cache côté
  noyau (`MODULE_CACHE`) ; le REPL ne paie l'init qu'une fois par session.
- **Limites** : pas d'extensions C (numpy...), pas de `socket` (WASI p1 sans
  sockets). `pip` est implémenté côté noyau (`src/lang/pip.rs`) : PyPI via la
  pile TLS maison, wheels pures `py3-none-any` uniquement.
