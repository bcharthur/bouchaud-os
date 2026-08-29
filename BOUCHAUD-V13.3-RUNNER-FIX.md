# Bouchaud OS — V13.3 runner false-negative fix

## Diagnostic confirmé

`cargo check` termine avec succès et `cmd /c "cargo bootimage > ... 2>&1"`
produit également un bootimage valide.

Le problème n'est donc plus le noyau V13.2. Il est dans `run.ps1` :

```powershell
$ErrorActionPreference = "Stop"
...
cargo bootimage
```

Cargo écrit ses messages de progression et ses warnings sur stderr même lorsque
son code de sortie vaut 0. Windows PowerShell transforme ces lignes stderr en
`NativeCommandError`; avec `ErrorActionPreference=Stop`, le script est interrompu
avant le test `$LASTEXITCODE`.

## Correctif sûr

Cette archive ajoute :

```text
tools/run/cargo-bootimage-safe.cmd
```

Le wrapper fusionne stderr vers stdout *à l'intérieur de cmd.exe*, puis transmet
le vrai code de sortie de Cargo.

Dans `run.ps1`, remplacer uniquement :

```powershell
cargo bootimage
```

par :

```powershell
& "$RepoRoot\tools\run\cargo-bootimage-safe.cmd"
```

Le bloc suivant reste inchangé :

```powershell
if ($LASTEXITCODE -ne 0) {
    Fail "cargo bootimage a echoue (code $LASTEXITCODE). QEMU ne sera pas lance."
}
```

Ainsi :
- warnings Cargo visibles ;
- pas de faux `NativeCommandError` ;
- vrai code de sortie Cargo conservé ;
- QEMU se lance seulement si le bootimage a réellement réussi.
