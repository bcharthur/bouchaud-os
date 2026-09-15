param()
$ErrorActionPreference = "Stop"
if (-not (Test-Path ".\Cargo.toml")) { throw "Lance ce script depuis la racine du depot." }

$checks = @(
    @("src/drivers/network/e1000.rs", "RX_INTERRUPTS_ACTIVE"),
    @("src/drivers/network/e1000.rs", "REG_IMS"),
    @("src/drivers/network/e1000.rs", "REG_ITR"),
    @("src/arch/x86_64/interrupts.rs", "Network = PIC_1_OFFSET + 11"),
    @("src/arch/x86_64/idt.rs", "network_interrupt_handler"),
    @("src/compat/linux/file.rs", "POLL_SOCKETS_WATCHDOG_NS")
)
foreach ($c in $checks) {
    $txt = Get-Content -Raw $c[0]
    if (-not $txt.Contains($c[1])) { throw "Contrat absent: $($c[1]) dans $($c[0])" }
}

git diff --check
if ($LASTEXITCODE -ne 0) { throw "git diff --check KO" }

.\tools\dev\validate-fast.ps1 -Bootimage
if ($LASTEXITCODE -ne 0) { throw "validate-fast KO" }
Write-Host "`nVALIDATION VERTE." -ForegroundColor Green
