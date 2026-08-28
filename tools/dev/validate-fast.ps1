param(
    [switch]$Bootimage
)

$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$Tmp = Join-Path $env:TEMP ("bouchaud-fast-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $Tmp | Out-Null

$results = [System.Collections.Generic.List[object]]::new()
$oldLocation = Get-Location
$oldHello = $env:BO_HELLO_EXE

function Add-Result([string]$Name, [bool]$Ok, [double]$Seconds, [string]$Detail = "") {
    $script:results.Add([pscustomobject]@{
        Test = $Name
        Status = if ($Ok) { "PASS" } else { "FAIL" }
        Seconds = [math]::Round($Seconds, 2)
        Detail = $Detail
    }) | Out-Null
}

function Invoke-Step([string]$Name, [scriptblock]$Action) {
    Write-Host "`n=== $Name ===" -ForegroundColor Cyan
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        & $Action
        if ($LASTEXITCODE -ne $null -and $LASTEXITCODE -ne 0) {
            throw "$Name a retourne le code $LASTEXITCODE"
        }
        $sw.Stop()
        Add-Result $Name $true $sw.Elapsed.TotalSeconds
    }
    catch {
        $sw.Stop()
        Add-Result $Name $false $sw.Elapsed.TotalSeconds $_.Exception.Message
        throw
    }
}

function Invoke-RustTest([string]$Name, [string]$Source, [switch]$SingleThread) {
    $exe = Join-Path $Tmp (([IO.Path]::GetFileNameWithoutExtension($Source)) + ".exe")
    Invoke-Step "$Name / compile" {
        & rustc --edition 2021 --test -o $exe $Source
    }
    Invoke-Step $Name {
        if ($SingleThread) {
            & $exe --test-threads=1
        } else {
            & $exe
        }
    }
}

try {
    Set-Location $Root
    Write-Host "Bouchaud OS - validation rapide" -ForegroundColor Green
    Write-Host "repo   : $Root"
    Write-Host "commit : $(& git rev-parse --short HEAD)"

    Invoke-RustTest "clavier" "tools\gui\test_clavier.rs"
    Invoke-RustTest "damage" "tools\gui\test_degats.rs" -SingleThread
    Invoke-RustTest "geometrie du bureau" "tools\gui\test_disposition.rs"
    Invoke-RustTest "chaine entree -> LFB" "tools\gui\test_chaine.rs"
    Invoke-RustTest "protocole GUI" "tools\gui\test_protocole.rs"
    Invoke-RustTest "compositeur event-driven" "tools\gui\test_reveil.rs"
    Invoke-RustTest "culling de scene" "tools\gui\test_scene.rs"
    Invoke-RustTest "equivalence de rendu" "tools\gui\test_rendu.rs"
    Invoke-RustTest "oracle de transition d'etat" "tools\gui\test_transitions.rs"
    Invoke-RustTest "systeme de fenetrage" "tools\gui\test_fenetrage.rs"
    Invoke-RustTest "decoupe du texte" "tools\gui\test_texte.rs"
    Invoke-RustTest "lots du port serie" "tools\serie\test_lots.rs"
    Invoke-RustTest "decodeur PNG" "tools\gui\test_png.rs"
    Invoke-RustTest "echeances de reveil" "tools\smp\test_echeances.rs"
    Invoke-RustTest "verdict protocole client" "tools\gui\test_silence.rs"
    Invoke-RustTest "BKL max/provenance" "tools\smp\test_bkl_max.rs"
    Invoke-RustTest "commutation SMP" "tools\smp\test_commutation.rs"
    Invoke-RustTest "profondeur BKL" "tools\smp\test_profondeur_bkl.rs"
    Invoke-RustTest "re-entree IRQ runqueue" "tools\smp\test_runqueue_irq.rs"
    Invoke-RustTest "cout des frames libres" "tools\smp\test_frames_libres.rs"
    Invoke-RustTest "cout des frames possedees" "tools\smp\test_pages_possedees.rs"
    Invoke-RustTest "cache de pages propres" "tools\smp\test_cache_pages.rs"
    Invoke-RustTest "discipline du gros verrou" "tools\smp\test_discipline_bkl.rs"
    Invoke-RustTest "ordre des verrous du cache" "tools\smp\test_ordre_verrous.rs"

    $env:BO_HELLO_EXE = Join-Path $Tmp "bo-hello.exe"
    Invoke-Step "fixture PE32+" {
        & python .\tools\exec\fabrique-hello-exe.py $env:BO_HELLO_EXE
    }
    Invoke-RustTest "formats ELF/PE" "tools\exec\test_format.rs"
    Invoke-RustTest "preparation d'image" "tools\exec\test_image.rs"

    Invoke-Step "manifeste polices" {
        & python .\tools\gui\verifie-polices.py
    }

    Invoke-Step "ordre des verrous (source)" {
        & python .\tools\verifie-ordre-verrous.py
    }

    Invoke-Step "ouverture de fenetre (source)" {
        & python .\tools\verifie-ouverture-fenetre.py
    }

    Invoke-Step "mutation d'etat de fenetre (source)" {
        & python .\tools\verifie-mutations-fenetre.py
    }

    Invoke-Step "ecriture incrementale de /persist (source)" {
        & python .\tools\verifie-persistance.py
    }

    Invoke-Step "icones du bureau (reproductibles)" {
        & python .\tools\assets\fabrique-icones.py --verifie
    }

    Invoke-Step "jeu de caracteres des pages" {
        & python .\tools\userland\navigateur\test_charset.py
    }

    Invoke-Step "polices du navigateur (source)" {
        & python .\tools\verifie-polices-navigateur.py
    }

    Invoke-Step "polices du Web (fontconfig)" {
        & python .\tools\verifie-polices-web.py
    }

    Invoke-Step "atlas de glyphes du chrome" {
        & python .\tools\ladybird\chrome\test_atlas.py
    }

    Invoke-Step "atlas du chrome (reproductible)" {
        & python .\tools\ladybird\chrome\fabrique-atlas.py --verifie
    }

    Invoke-Step "echeances de reveil (source)" {
        & python .\tools\verifie-echeances.py
    }

    if (Test-Path ".\tools\verifie-protocole-gui.py") {
        Invoke-Step "coherence protocole GUI" {
            & python .\tools\verifie-protocole-gui.py
        }
    }

    Invoke-Step "git diff --check" {
        & git diff --check
    }

    Invoke-Step "cargo check" {
        & cargo check
    }

    if ($Bootimage) {
        Invoke-Step "cargo bootimage" {
            & cargo bootimage
        }
    }
}
catch {
    Write-Host "`nECHEC: $($_.Exception.Message)" -ForegroundColor Red
}
finally {
    if ($oldHello) { $env:BO_HELLO_EXE = $oldHello } else { Remove-Item Env:BO_HELLO_EXE -ErrorAction SilentlyContinue }
    Set-Location $oldLocation
    Remove-Item $Tmp -Recurse -Force -ErrorAction SilentlyContinue

    Write-Host "`n=== RESUME ===" -ForegroundColor Cyan
    $results | Format-Table -AutoSize
    $failed = @($results | Where-Object Status -eq "FAIL")
    if ($failed.Count -gt 0) {
        Write-Host "VALIDATION: ECHEC ($($failed.Count) etape(s))" -ForegroundColor Red
        exit 1
    }
    Write-Host "VALIDATION: OK" -ForegroundColor Green
    exit 0
}
