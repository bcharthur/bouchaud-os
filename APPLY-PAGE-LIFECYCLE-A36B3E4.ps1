$ErrorActionPreference = 'Stop'

$Root = $PSScriptRoot
$ExpectedBase = 'a36b3e477adedeaf807992b6a75ea186a5c64271'
$RunnerRelative = 'tools/ladybird/browser-upstream.sh'
$Runner = Join-Path $Root $RunnerRelative
$Hook = 'python3 tools/ladybird/prepare-m11-page-registry.py "$SRC"'
$Anchor = 'python3 tools/ladybird/prepare-v19-navigateur.py "$SRC"'

if (-not (Test-Path (Join-Path $Root '.git'))) {
    throw "Extrais le ZIP a la racine du depot bouchaud-os puis relance ce script."
}

$Head = (git -C $Root rev-parse HEAD).Trim()
$Branch = (git -C $Root branch --show-current).Trim()
Write-Host "BRANCHE : $Branch"
Write-Host "HEAD    : $Head"

$RunnerText = [System.IO.File]::ReadAllText($Runner)
if (-not $RunnerText.Contains($Hook) -and $Head -ne $ExpectedBase) {
    throw "Base inattendue. Attendu $ExpectedBase, obtenu $Head. Je refuse de patcher a l'aveugle."
}

# Ne jamais ecraser une modification locale du seul fichier existant que nous
# allons toucher. Les deux scripts Python du ZIP sont nouveaux et visibles dans
# git status ; ils ne justifient aucun reset/clean/restore.
if (-not $RunnerText.Contains($Hook)) {
    git -C $Root diff --quiet -- $RunnerRelative
    if ($LASTEXITCODE -ne 0) {
        throw "$RunnerRelative a deja des modifications locales. Aucun ecrasement effectue."
    }

    if (-not $RunnerText.Contains($Anchor)) {
        throw "Ancre prepare-v19-navigateur.py introuvable dans $RunnerRelative"
    }

    $NewLine = if ($RunnerText.Contains("`r`n")) { "`r`n" } else { "`n" }
    $RunnerText = $RunnerText.Replace($Anchor, $Anchor + $NewLine + $Hook)
    $Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($Runner, $RunnerText, $Utf8NoBom)
    Write-Host "Hook ajoute apres prepare-v19-navigateur.py"
} else {
    Write-Host "Hook deja present : rien a reecrire"
}

python (Join-Path $Root 'tools/verifie-lifecycle-pages.py')
if ($LASTEXITCODE -ne 0) { throw 'Garde lifecycle en echec' }

python -m py_compile `
  (Join-Path $Root 'tools/ladybird/prepare-m11-page-registry.py') `
  (Join-Path $Root 'tools/verifie-lifecycle-pages.py')
if ($LASTEXITCODE -ne 0) { throw 'Compilation Python en echec' }

Write-Host ''
Write-Host 'PATCH APPLIQUE. Diff a inspecter :' -ForegroundColor Green
git -C $Root status --short
Write-Host ''
Write-Host 'Ensuite :'
Write-Host '  python tools/verifie-lifecycle-pages.py'
Write-Host '  git diff --check'
Write-Host '  powershell -ExecutionPolicy Bypass -File .\tools\reference\IMAGE-TRIGKEY.ps1'
Write-Host ''
Write-Host 'Ne commit/push rien avant le test QEMU/build.'
