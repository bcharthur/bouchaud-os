# BOUCHAUD_LADYBIRD_ARTIFACT_V7
# Windows PowerShell 5.1. Browser is linked to build inputs, never local branch name.
function Invoke-BoExternal {
    param([string]$Root, [string]$Program, [string[]]$Params)
    $before = (Get-Location).Path
    $oldPreference = $ErrorActionPreference
    try {
        Set-Location -LiteralPath $Root
        $ErrorActionPreference = 'Continue'
        $result = @(& $Program @Params 2>$null)
        $status = [int]$LASTEXITCODE
    }
    finally {
        Set-Location -LiteralPath $before
        $ErrorActionPreference = $oldPreference
    }
    return [pscustomobject]@{ Code = $status; Text = ($result -join "`n") }
}
function Get-BoFingerprint {
    param([string]$Root, [string]$Revision = 'HEAD')
    if ($Revision -ne 'HEAD' -and $Revision -notmatch '^[0-9a-f]{40}$') { return $null }
    if ($Revision -ne 'HEAD') {
        $has = Invoke-BoExternal -Root $Root -Program 'git.exe' -Params @('cat-file','-e',($Revision + '^{commit}'))
        if ($has.Code -ne 0) {
            $fetch = Invoke-BoExternal -Root $Root -Program 'git.exe' -Params @('fetch','--no-tags','--depth=1','origin',$Revision)
            if ($fetch.Code -ne 0) { return $null }
        }
    }
    $path = Join-Path $Root 'tools/ci/ladybird_empreinte.py'
    $out = Invoke-BoExternal -Root $Root -Program 'python' -Params @($path,'calcule','--commit',$Revision)
    if ($out.Code -ne 0) { return $null }
    $m = [regex]::Match($out.Text, 'LADYBIRD_EMPREINTE=([0-9a-f]{64})')
    if (-not $m.Success) { return $null }
    return $m.Groups[1].Value
}
# Pure selection with injected test delegates. Production uses real gh + Git.
function Get-BoCompatibleRuns {
    param([string]$Root, [string]$Fingerprint, [string]$ArtifactName,
          [long]$ExplicitRunId = 0, [string]$Repository = 'bcharthur/bouchaud-os',
          [scriptblock]$GhCall = $null, [scriptblock]$FingerprintCall = $null)
    if ($Fingerprint -notmatch '^[0-9a-f]{64}$') { throw 'Empreinte Ladybird invalide' }
    $runs = @()
    if ($ExplicitRunId -gt 0) {
        $runs = @([pscustomobject]@{databaseId=$ExplicitRunId;headSha='';event=''})
    }
    else {
        $argsList = @('run','list','-R',$Repository,'--workflow','ladybird-native-browser.yml',
                      '--limit','100','--json','databaseId,headSha,event')
        $resp = if ($GhCall) { & $GhCall $argsList } else { Invoke-BoExternal -Root $Root -Program 'gh' -Params $argsList }
        if ($resp.Code -ne 0) { throw 'Impossible de consulter les runs GitHub Ladybird (gh run list)' }
        if ($resp.Text.Trim()) {
            $decoded = ConvertFrom-Json -InputObject $resp.Text -ErrorAction Stop
            $runs = @($decoded)
        }
    }
    $seen = @{}
    $selected = New-Object System.Collections.ArrayList
    foreach ($entry in $runs) {
        if ($null -eq $entry -or $null -eq $entry.databaseId) { continue }
        $id = [long]$entry.databaseId
        if ($id -le 0 -or $seen.ContainsKey([string]$id)) { continue }
        $seen[[string]$id] = $true
        if ($entry.event -and $entry.event -notin @('push','workflow_dispatch')) { continue }
        $apiRun = @('api',('repos/{0}/actions/runs/{1}' -f $Repository,$id))
        $meta = if ($GhCall) { & $GhCall $apiRun } else { Invoke-BoExternal -Root $Root -Program 'gh' -Params $apiRun }
        if ($meta.Code -ne 0) { continue }
        try { $info = ConvertFrom-Json -InputObject $meta.Text -ErrorAction Stop }
        catch { continue }
        if ($null -eq $info.repository -or $null -eq $info.head_repository) { continue }
        if ($info.repository.full_name -ne $Repository -or
            $info.head_repository.full_name -ne $Repository -or
            $info.event -notin @('push','workflow_dispatch') -or
            $info.path -ne '.github/workflows/ladybird-native-browser.yml') { continue }
        $sha = [string]$info.head_sha
        if ($sha -notmatch '^[0-9a-f]{40}$') { continue }
        if ($entry.headSha -and $entry.headSha -ne $sha) { continue }
        $apiArtifacts = @('api',('repos/{0}/actions/runs/{1}/artifacts?per_page=100' -f $Repository,$id))
        $art = if ($GhCall) { & $GhCall $apiArtifacts } else { Invoke-BoExternal -Root $Root -Program 'gh' -Params $apiArtifacts }
        if ($art.Code -ne 0) { continue }
        try { $data = ConvertFrom-Json -InputObject $art.Text -ErrorAction Stop }
        catch { continue }
        $valid = @($data.artifacts | Where-Object {
            $_.name -eq $ArtifactName -and $_.expired -eq $false -and $_.size_in_bytes -gt 0
        })
        if ($valid.Count -eq 0) { continue }
        $producer = if ($FingerprintCall) { & $FingerprintCall $sha } else { Get-BoFingerprint -Root $Root -Revision $sha }
        if (-not $producer -or $producer -ne $Fingerprint) { continue }
        [void]$selected.Add([pscustomobject]@{RunId=$id;HeadSha=$sha})
        if ($selected.Count -ge 3) { break }
    }
    foreach ($item in $selected) { Write-Output $item }
}
function Test-BoArtifact {
    param([string]$Root, [string]$Directory, [string[]]$RequiredFiles)
    if (-not (Test-Path -LiteralPath $Directory -PathType Container)) { return $false }
    foreach ($name in $RequiredFiles) {
        if (-not (Test-Path -LiteralPath (Join-Path $Directory $name) -PathType Leaf)) { return $false }
    }
    if (-not (Test-Path -LiteralPath (Join-Path $Directory 'resources') -PathType Container)) { return $false }
    $manifest = Join-Path $Directory 'BOUCHAUD_ARTIFACT_MANIFEST.json'
    if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) { return $false }
    $verifier = Join-Path $Root 'tools/ladybird/artifact_manifest.py'
    $upstream = Join-Path $Root 'third_party/UPSTREAM.md'
    $checked = Invoke-BoExternal -Root $Root -Program 'python' -Params @($verifier,'verify',
        '--root',$Directory,'--upstream',$upstream,'--manifest',$manifest,'--allow-head-mismatch')
    if ($checked.Code -ne 0) { return $false }
    $link = Join-Path $Root 'tools/ci/ladybird_empreinte.py'
    $match = Invoke-BoExternal -Root $Root -Program 'python' -Params @($link,'lie','--manifest',$manifest)
    return ($match.Code -eq 0)
}
function Use-BoLadybirdArtifact {
    param([string]$RepoRoot,[string]$TargetDir,[string[]]$RequiredFiles,
          [string]$ArtifactName,[long]$RunId=0,[switch]$Refresh)
    if (-not (Get-Command 'git.exe' -ErrorAction SilentlyContinue)) { throw 'Ladybird : git.exe absent' }
    $current = Get-BoFingerprint -Root $RepoRoot -Revision 'HEAD'
    if (-not $current) { throw 'Ladybird : empreinte des sources du HEAD indisponible' }
    $goodCache = Test-BoArtifact -Root $RepoRoot -Directory $TargetDir -RequiredFiles $RequiredFiles
    if ($goodCache -and -not $Refresh -and $RunId -eq 0) {
        Write-Host ('Ladybird : cache verifie (recette {0})' -f $current.Substring(0,16)) -ForegroundColor Green
        return
    }
    if (-not (Get-Command 'gh' -ErrorAction SilentlyContinue)) {
        if ($goodCache -and $RunId -eq 0) { Write-Host 'Ladybird : cache conserve, gh absent' -ForegroundColor Yellow; return }
        throw 'Ladybird : GitHub CLI absente (installer gh, puis gh auth login)'
    }
    Write-Host ('Ladybird : recherche par empreinte {0}, pas par branche' -f $current.Substring(0,16)) -ForegroundColor Cyan
    $runs = @(Get-BoCompatibleRuns -Root $RepoRoot -Fingerprint $current -ArtifactName $ArtifactName -ExplicitRunId $RunId)
    if ($runs.Count -eq 0) {
        if ($goodCache -and $RunId -eq 0) { Write-Host 'Ladybird : cache verifie conserve' -ForegroundColor Yellow; return }
        throw 'Ladybird : aucun artefact de compilation compatible et non expire disponible'
    }
    $suffix = [guid]::NewGuid().ToString('N')
    $stage = $TargetDir + '.download-' + $suffix
    $backup = $TargetDir + '.backup-' + $suffix
    $installed = $false
    try {
        foreach ($run in $runs) {
            if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
            New-Item -ItemType Directory -Path $stage | Out-Null
            Write-Host ('Ladybird : run {0} (SHA {1})' -f $run.RunId,$run.HeadSha.Substring(0,12)) -ForegroundColor Cyan
            $dl = Invoke-BoExternal -Root $RepoRoot -Program 'gh' -Params @('run','download',
                [string]$run.RunId,'-R','bcharthur/bouchaud-os','-n',$ArtifactName,'-D',$stage)
            if ($dl.Code -ne 0) { continue }
            if (-not (Test-BoArtifact -Root $RepoRoot -Directory $stage -RequiredFiles $RequiredFiles)) { continue }
            $old = Test-Path -LiteralPath $TargetDir
            if ($old) { Move-Item -LiteralPath $TargetDir -Destination $backup -ErrorAction Stop }
            try {
                Move-Item -LiteralPath $stage -Destination $TargetDir -ErrorAction Stop
                $installed = $true
            }
            catch {
                if ($old -and (Test-Path -LiteralPath $backup) -and -not (Test-Path -LiteralPath $TargetDir)) {
                    Move-Item -LiteralPath $backup -Destination $TargetDir -ErrorAction Stop
                }
                throw
            }
            if (Test-Path -LiteralPath $backup) { Remove-Item -LiteralPath $backup -Recurse -Force }
            break
        }
    }
    finally {
        if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
    }
    if (-not $installed) {
        if ($goodCache -and $RunId -eq 0) { Write-Host 'Ladybird : echec reseau, cache verifie conserve' -ForegroundColor Yellow; return }
        throw 'Ladybird : aucun telechargement ne passe la verification du manifeste et de la recette'
    }
    Write-Host 'Ladybird : artefact compatible installe et verifie' -ForegroundColor Green
}
