# Bouchaud OS — V13.1 compile fix

## Erreur corrigée

`cargo check` remontait une seule erreur bloquante :

```text
error[E0597]: `process` does not live long enough
 --> src/kernel/sync/wait_word/cle.rs
```

La cause est une durée de vie de temporaire Rust :

```rust
process.mm.lock().space.translate(uaddr).or(Some(uaddr))
```

Placée directement comme expression de retour, cette chaîne garde le
`SpinLockGuard` temporaire vivant jusqu'à la destruction de l'expression finale,
alors que `process` est déjà en cours de destruction.

V13.1 introduit un scope explicite :

```rust
let translated = {
    let mm = process.mm.lock();
    mm.space.translate(uaddr)
};

translated.or(Some(uaddr))
```

Le guard est donc détruit avant `process`.

## Portée

Aucun rollback du grand V13 :
- WaitWord natif conservé ;
- buckets locaux conservés ;
- WaitSource ciblé conservé ;
- persistance transactionnelle conservée ;
- readahead conservé ;
- desktop/event-driven conservé.

Le vérificateur V13 est également renforcé pour contrôler :
- le scope du guard `wait_word_key`;
- la présence de `WaitSource::signal_one`;
- l'utilisation du wake ciblé par WaitWord.

## Test

Appliquer ce ZIP directement par-dessus V13, puis :

```powershell
python .\tools\dev\verifie-v13.py
python .\tools\dev\verifie-v13.1.py

$env:CARGO_TERM_COLOR="never"
cargo check 2>&1 | Tee-Object -FilePath v13.1-cargo-check.log
```

Si `cargo check` passe :

```powershell
.\run.ps1 -Ladybird -LadybirdUrl "https://www.google.com/" 2>&1 |
    Tee-Object -FilePath v13.1-tcg-smp.log
```
