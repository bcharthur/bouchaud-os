# Bouchaud OS — V13.2 compile fix

## Erreur corrigée

Après V13.1, `cargo check` ne remonte plus E0597 mais une seule erreur :

```text
error[E0596]: cannot borrow `mm` as mutable, as it is not declared as mutable
 --> src/kernel/sync/wait_word/cle.rs
```

`AddressSpace::translate()` demande un accès mutable. Le `SpinLockGuard`
explicitement scopé par V13.1 doit donc être déclaré mutable :

```rust
let translated = {
    let mut mm = process.mm.lock();
    mm.space.translate(uaddr)
};

translated.or(Some(uaddr))
```

## Portée

Aucun changement d'algorithme ou d'architecture. V13.2 est un correctif de
compilation strict :
- WaitWord natif inchangé ;
- buckets locaux inchangés ;
- persistance transactionnelle inchangée ;
- readahead inchangé ;
- Event-Driven Core inchangé.

Les vérificateurs V13/V13.1 sont renforcés et un `verifie-v13.2.py` est ajouté.

## Test

Appliquer ce ZIP par-dessus V13.1 :

```powershell
python .\tools\dev\verifie-v13.py
python .\tools\dev\verifie-v13.1.py
python .\tools\dev\verifie-v13.2.py

$env:CARGO_TERM_COLOR="never"
cargo check 2>&1 | Tee-Object -FilePath v13.2-cargo-check.log
```

Si `cargo check` termine sur `Finished`, lancer ensuite le run SMP complet.
