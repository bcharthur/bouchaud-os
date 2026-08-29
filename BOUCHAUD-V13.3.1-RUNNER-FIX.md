# Bouchaud OS — V13.3.1 runner fix

Le noyau V13.2 est valide :
- `cargo check` passe ;
- `cargo bootimage` passe ;
- le bootloader est construit ;
- `bootimage-bouchaud-os.bin` est créé.

Le problème restant est le runner PowerShell.

## Défaut de V13.3

Le fichier `run.ps1.patch` de V13.3 était syntaxiquement invalide :

```diff
@@
-cargo bootimage
+...
```

Un unified diff doit porter des coordonnées de hunk (`@@ -a,b +c,d @@`).
`git apply` répondait donc :

```text
error: No valid patches in input
```

## V13.3.1

Le patch est maintenant un vrai unified diff :

```diff
@@ -292,7 +292,7 @@
 Write-Section "Construction du noyau"

 Ensure-Cargo

-cargo bootimage
+& "$RepoRoot\tools\run\cargo-bootimage-safe.cmd"
```

Le wrapper conserve le vrai code de sortie de Cargo tout en fusionnant stderr
vers stdout à l'intérieur de `cmd.exe`, afin que `$ErrorActionPreference="Stop"`
ne transforme plus les warnings Cargo en faux échec.

## Application

```powershell
git apply --check .\run.ps1.patch
git apply .\run.ps1.patch

.\tools\dev\verifie-v13.3.1.ps1
```

Puis :

```powershell
.\run.ps1 -Ladybird -LadybirdUrl "https://www.google.com/" 2>&1 |
    Tee-Object -FilePath v13.3.1-tcg-smp.log
```
