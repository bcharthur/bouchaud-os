#requires -Version 5.1
param([string]$RepoRoot = '')
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $RepoRoot) { $RepoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot) }
. (Join-Path $RepoRoot 'tools/run/ladybird-artifact.ps1')

$script:Runs = @{
  '101' = @{ Event='pull_request'; Sha=('a'*40); Expired=$false }
  '102' = @{ Event='push'; Sha=('b'*40); Expired=$false }
  '103' = @{ Event='push'; Sha=('c'*40); Expired=$true }
  '104' = @{ Event='push'; Sha=('d'*40); Expired=$false }
}
$script:None = $false
$mockGh = {
    param([string[]]$callArgs)
    if ($callArgs[0] -eq 'run' -and $callArgs[1] -eq 'list') {
        $ids = if ($script:None) { @() } else { @(101,102,103,104) }
        $entries = @()
        foreach ($id in $ids) {
            $e = $script:Runs[[string]$id]
            $entries += [pscustomobject]@{databaseId=$id;headSha=$e.Sha;event=$e.Event}
        }
        $json = ConvertTo-Json -InputObject $entries -Depth 5 -Compress
        return [pscustomobject]@{Code=0;Text=[string]$json}
    }
    if ($callArgs[0] -eq 'api') {
        $api = [string]$callArgs[1]
        if ($api -notmatch '^repos/bcharthur/bouchaud-os/actions/runs/([0-9]+)') {
            throw "unexpected gh api $api"
        }
        $id = [string]$Matches[1]
        $e = $script:Runs[$id]
        if (-not $e) { return [pscustomobject]@{Code=1;Text='missing'} }
        if ($api.Contains('/artifacts')) {
            $obj = @{artifacts=@(@{name='bouchaud-ladybird-native-browser';expired=$e.Expired;size_in_bytes=1024})}
        }
        else {
            $obj = @{repository=@{full_name='bcharthur/bouchaud-os'};
                     head_repository=@{full_name='bcharthur/bouchaud-os'};
                     event=$e.Event;path='.github/workflows/ladybird-native-browser.yml';head_sha=$e.Sha}
        }
        return [pscustomobject]@{Code=0;Text=(ConvertTo-Json -InputObject $obj -Depth 5 -Compress)}
    }
    throw ('unexpected gh: '+($callArgs -join ' '))
}
$mockFingerprint = {
    param([string]$sha)
    if ($sha -eq ('b'*40)) { return ('b'*64) }
    return ('a'*64)
}
function Assert-Case([bool]$passed,[string]$name) {
    if (-not $passed) { throw "LADYBIRD_V7_TEST_FAIL $name" }
    Write-Host "LADYBIRD_V7_TEST_OK $name"
}
function Search-Runs([long]$explicit) {
    return @(Get-BoCompatibleRuns -Root $RepoRoot -Fingerprint ('a'*64) `
        -ArtifactName 'bouchaud-ladybird-native-browser' -ExplicitRunId $explicit `
        -GhCall $mockGh -FingerprintCall $mockFingerprint)
}
$found = @(Search-Runs 0)
Write-Host ('DIAGNOSTIC candidates='+($found | ForEach-Object { $_.RunId } | Out-String).Trim())
Assert-Case ($found.Count -eq 1 -and [long]$found[0].RunId -eq 104) 'reject PR, mismatched recipe, expired artifact'
$script:None = $true
$found = @(Search-Runs 0)
Assert-Case ($found.Count -eq 0) 'empty GitHub run list'
$script:None = $false
$found = @(Search-Runs 101)
Assert-Case ($found.Count -eq 0) 'reject explicit pull_request run'
$found = @(Search-Runs 102)
Assert-Case ($found.Count -eq 0) 'reject explicit different build recipe'
$found = @(Search-Runs 103)
Assert-Case ($found.Count -eq 0) 'reject expired artifact'
$found = @(Search-Runs 104)
Assert-Case ($found.Count -eq 1 -and [long]$found[0].RunId -eq 104) 'accept explicit first-party compatible run'
$dummy = Join-Path ([System.IO.Path]::GetTempPath()) ('bo-artifact-v7-'+[guid]::NewGuid().ToString('N'))
try {
    New-Item -ItemType Directory -Path $dummy | Out-Null
    Assert-Case (-not (Test-BoArtifact -Root $RepoRoot -Directory $dummy -RequiredFiles @('WebContent'))) 'cache without manifest rejected'
}
finally {
    if (Test-Path -LiteralPath $dummy) { Remove-Item -LiteralPath $dummy -Recurse -Force }
}
Write-Host 'LADYBIRD_V7_TEST_PASS 7 offline scenarios on exact PowerShell engine' -ForegroundColor Green

exit 0
