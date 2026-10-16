# Lecture SEULE des propriétés Windows de la machine sur laquelle on lance ceci.
# A lancer SUR LE TRIGKEY avant d'éteindre Windows, pas sur le PC développeur.
[CmdletBinding()]
param([string]$Output = (Join-Path (Get-Location).Path 'trigkey-inventaire.json'))
$ErrorActionPreference = 'Stop'
$machine = Get-CimInstance Win32_ComputerSystem | Select-Object Manufacturer, Model, TotalPhysicalMemory
$cpu = @(Get-CimInstance Win32_Processor | Select-Object Name, NumberOfCores, NumberOfLogicalProcessors)
$bios = Get-CimInstance Win32_BIOS | Select-Object Manufacturer, Version
$disk = @(Get-Disk | Select-Object Number, FriendlyName, BusType, Size, PartitionStyle)
$network = @(Get-NetAdapter | Select-Object Name, InterfaceDescription, Status, LinkSpeed)
$display = @(Get-PnpDevice -Class Display -PresentOnly -ErrorAction SilentlyContinue | Select-Object FriendlyName, Status)
$inputDevices = @(Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue |
    Where-Object { $_.Class -in @('Keyboard', 'Mouse') } |
    Select-Object Class, FriendlyName, Status)
$payload = [pscustomobject]@{
    gathered_at = (Get-Date).ToString('o')
    device = $machine
    cpu = $cpu
    bios = $bios
    disks_read_only = $disk
    network = $network
    display = $display
    input_devices = $inputDevices
    warning = 'Inventaire Windows seulement : ne prouve PAS que Bouchaud OS possède les pilotes'
}
$destination = [System.IO.Path]::GetFullPath($Output)
if (Test-Path -LiteralPath $destination) { throw "Refus d'écraser un inventaire existant : $destination" }
$payload | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $destination -Encoding UTF8
Write-Host "TRIGKEY_INVENTAIRE_ECRIT $destination" -ForegroundColor Green
Write-Host 'Aucun disque, partition, périphérique ou firmware modifié.'
