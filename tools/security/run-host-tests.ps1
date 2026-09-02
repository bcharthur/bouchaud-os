$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$Out = Join-Path $Root "target\security-host"
New-Item -ItemType Directory -Force -Path $Out | Out-Null

& rustc --test `
    (Join-Path $Root "tools\security\test_security_policy.rs") `
    -o (Join-Path $Out "security-policy-tests.exe")
if ($LASTEXITCODE -ne 0) { throw "Compilation des tests security impossible" }

& (Join-Path $Out "security-policy-tests.exe")
if ($LASTEXITCODE -ne 0) { throw "Tests security en echec" }

Write-Host "SECURITY_HOST_OK"
