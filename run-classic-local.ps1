param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [object[]]$ExtraArgs
)

$ErrorActionPreference = "Stop"
$RunScript = Join-Path $PSScriptRoot "run.ps1"

if (-not (Test-Path -LiteralPath $RunScript -PathType Leaf)) {
    throw "run.ps1 introuvable : $RunScript"
}

& $RunScript @ExtraArgs
exit $LASTEXITCODE
