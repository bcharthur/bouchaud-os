# BOUCHAUD_P13_1_V4_HOST_FINAL
param(
    [string]$PublicInterface = "Wi-Fi",
    [string]$PrivateInterface = "Ethernet",
    [string]$LabSourceIp = "169.254.6.185",
    [int]$LabPrefixLength = 16,
    [string]$IcsPrivateIp = "192.168.137.1",
    [string]$TargetIp = "169.254.178.21",
    [int]$TargetPort = 2222,
    [int]$ReadyTimeoutSeconds = 15,
    [switch]$ForceRecycle,
    [switch]$SkipTargetCheck
)

$ErrorActionPreference = "Stop"

function Assert-Admin {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    $p = New-Object Security.Principal.WindowsPrincipal($id)
    if (-not $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw "Lance ce script dans PowerShell en administrateur."
    }
}

function Get-IcsState {
    $share = New-Object -ComObject HNetCfg.HNetShare
    $all = @($share.EnumEveryConnection())
    $public = $all | Where-Object { ($share.NetConnectionProps($_)).Name -eq $PublicInterface } | Select-Object -First 1
    $private = $all | Where-Object { ($share.NetConnectionProps($_)).Name -eq $PrivateInterface } | Select-Object -First 1
    if (-not $public -or -not $private) {
        throw "Interface publique '$PublicInterface' ou privee '$PrivateInterface' introuvable."
    }
    [PSCustomObject]@{
        Share = $share
        All = $all
        Public = $public
        Private = $private
        PublicCfg = $share.INetSharingConfigurationForINetConnection($public)
        PrivateCfg = $share.INetSharingConfigurationForINetConnection($private)
    }
}

function Test-DhcpEndpoints {
    $ports = @(Get-NetUDPEndpoint -ErrorAction SilentlyContinue |
        Where-Object { $_.LocalAddress -eq $IcsPrivateIp -and $_.LocalPort -in 67,68 })
    $has67 = @($ports | Where-Object LocalPort -eq 67).Count -gt 0
    $has68 = @($ports | Where-Object LocalPort -eq 68).Count -gt 0
    return ($has67 -and $has68)
}

function Wait-DhcpEndpoints {
    $deadline = (Get-Date).AddSeconds($ReadyTimeoutSeconds)
    do {
        if (Test-DhcpEndpoints) { return $true }
        Start-Sleep -Milliseconds 500
    } while ((Get-Date) -lt $deadline)
    return $false
}

function Ensure-LabAddress {
    $current = Get-NetIPAddress -InterfaceAlias $PrivateInterface -AddressFamily IPv4 -ErrorAction SilentlyContinue |
        Where-Object { $_.IPAddress -eq $LabSourceIp } |
        Select-Object -First 1

    if (-not $current) {
        New-NetIPAddress -InterfaceAlias $PrivateInterface -IPAddress $LabSourceIp `
            -PrefixLength $LabPrefixLength -SkipAsSource $false | Out-Null
    } else {
        # IMPORTANT P13.1 V4 : cette adresse doit participer normalement a la
        # resolution de voisinage. SkipAsSource=True a ete prouve physiquement
        # fautif : Windows gardait la route /16 mais terminait les SYN BRDP par
        # "Address resolution timeout/failure". Avec False, ARP + TCP/2222
        # repartent tout en laissant ICS ecouter sur 192.168.137.1:67/68.
        Set-NetIPAddress -InterfaceAlias $PrivateInterface -IPAddress $LabSourceIp `
            -SkipAsSource $false
    }
}

function Wait-LabAddressPreferred {
    $deadline = (Get-Date).AddSeconds($ReadyTimeoutSeconds)
    do {
        $address = Get-NetIPAddress -InterfaceAlias $PrivateInterface -AddressFamily IPv4 -ErrorAction SilentlyContinue |
            Where-Object { $_.IPAddress -eq $LabSourceIp } |
            Select-Object -First 1
        if ($address -and $address.AddressState -eq "Preferred" -and -not $address.SkipAsSource) {
            return $true
        }
        Start-Sleep -Milliseconds 500
    } while ((Get-Date) -lt $deadline)
    return $false
}

function Save-LocalEnv {
    $repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
    $envFile = Join-Path $repo ".env"
    if (-not (Test-Path $envFile)) {
        [System.IO.File]::WriteAllText($envFile, "", (New-Object System.Text.UTF8Encoding($false)))
    }

    $lines = @(Get-Content $envFile -ErrorAction SilentlyContinue)
    $out = New-Object System.Collections.Generic.List[string]
    $replaced = $false
    foreach ($line in $lines) {
        if ($line -match '^\s*BOUCHAUD_LAB_SOURCE_IP\s*=') {
            if (-not $replaced) {
                $out.Add("BOUCHAUD_LAB_SOURCE_IP=$LabSourceIp")
                $replaced = $true
            }
        } else {
            $out.Add($line)
        }
    }
    if (-not $replaced) {
        if ($out.Count -gt 0 -and $out[$out.Count - 1] -ne "") { $out.Add("") }
        $out.Add("BOUCHAUD_LAB_SOURCE_IP=$LabSourceIp")
    }
    [System.IO.File]::WriteAllLines($envFile, $out, (New-Object System.Text.UTF8Encoding($false)))
}

function Test-TargetTcpFromLabSource {
    Remove-NetNeighbor -InterfaceAlias $PrivateInterface -IPAddress $TargetIp `
        -Confirm:$false -ErrorAction SilentlyContinue

    $client = $null
    try {
        $local = New-Object System.Net.IPEndPoint([System.Net.IPAddress]::Parse($LabSourceIp), 0)
        $client = New-Object System.Net.Sockets.TcpClient($local)
        $task = $client.ConnectAsync($TargetIp, $TargetPort)
        if (-not $task.Wait(5000)) {
            return $false
        }
        if ($task.IsFaulted -or -not $client.Connected) {
            return $false
        }
        return $true
    } catch {
        return $false
    } finally {
        if ($client) { $client.Close() }
    }
}

Assert-Admin

Write-Host "=== BOUCHAUD LAB HOST / ICS P13.1 V4 ===" -ForegroundColor Cyan
Write-Host "Public  : $PublicInterface" -ForegroundColor DarkGray
Write-Host "Prive   : $PrivateInterface" -ForegroundColor DarkGray
Write-Host "ICS IP  : $IcsPrivateIp" -ForegroundColor DarkGray
Write-Host "LAB IP  : $LabSourceIp/$LabPrefixLength (SkipAsSource=False)" -ForegroundColor DarkGray

$state = Get-IcsState
$needRecycle = $ForceRecycle -or -not (Test-DhcpEndpoints)

$publicOk = $state.PublicCfg.SharingEnabled -and $state.PublicCfg.SharingConnectionType -eq 0
$privateOk = $state.PrivateCfg.SharingEnabled -and $state.PrivateCfg.SharingConnectionType -eq 1
if (-not $publicOk -or -not $privateOk) { $needRecycle = $true }

if ($needRecycle) {
    Write-Host "ICS incoherent ou DHCP 67/68 absent : recyclage du partage..." -ForegroundColor Yellow

    foreach ($connection in $state.All) {
        $cfg = $state.Share.INetSharingConfigurationForINetConnection($connection)
        if ($cfg.SharingEnabled) { $cfg.DisableSharing() }
    }
    Start-Sleep -Seconds 2

    # Re-enumerer apres la bascule : les objets COM precedents peuvent etre perimes.
    $state = Get-IcsState
    $state.Share.INetSharingConfigurationForINetConnection($state.Public).EnableSharing(0)
    Start-Sleep -Seconds 1
    $state.Share.INetSharingConfigurationForINetConnection($state.Private).EnableSharing(1)

    if (-not (Wait-DhcpEndpoints)) {
        throw "ICS a ete recycle mais UDP 67/68 n'ecoute toujours pas sur $IcsPrivateIp."
    }
} else {
    Write-Host "ICS deja coherent et DHCP 67/68 present." -ForegroundColor Green
}

# ICS peut recreer l'interface privee. L'adresse LAB est donc posee APRES ICS.
Ensure-LabAddress
if (-not (Wait-LabAddressPreferred)) {
    throw "L'adresse LAB $LabSourceIp n'est pas Preferred avec SkipAsSource=False."
}

Save-LocalEnv
$env:BOUCHAUD_LAB_SOURCE_IP = $LabSourceIp

$icsIp = Get-NetIPAddress -InterfaceAlias $PrivateInterface -AddressFamily IPv4 -ErrorAction SilentlyContinue |
    Where-Object IPAddress -eq $IcsPrivateIp |
    Select-Object -First 1
if (-not $icsIp) {
    throw "ICS n'a pas pose $IcsPrivateIp sur $PrivateInterface."
}
if (-not (Test-DhcpEndpoints)) {
    throw "UDP 67/68 a disparu apres la pose de l'adresse LAB."
}

$targetOk = $null
if (-not $SkipTargetCheck) {
    Write-Host "Validation ARP/TCP BRDP depuis $LabSourceIp vers $TargetIp`:$TargetPort..." -ForegroundColor Cyan
    $targetOk = Test-TargetTcpFromLabSource
    if (-not $targetOk) {
        $neighbor = Get-NetNeighbor -InterfaceAlias $PrivateInterface -IPAddress $TargetIp -ErrorAction SilentlyContinue |
            Select-Object -First 1
        if ($neighbor) {
            Write-Host "Voisin $TargetIp : $($neighbor.State) / $($neighbor.LinkLayerAddress)" -ForegroundColor Yellow
        }
        throw "Host ICS pret, mais ARP/TCP BRDP vers $TargetIp`:$TargetPort echoue depuis $LabSourceIp."
    }
}

Write-Host ""
Write-Host "=== ETAT FINAL ===" -ForegroundColor Cyan
Get-Service SharedAccess | Format-Table Status,Name,StartType -AutoSize
Get-NetIPAddress -InterfaceAlias $PrivateInterface -AddressFamily IPv4 |
    Sort-Object IPAddress |
    Format-Table IPAddress,PrefixLength,AddressState,SkipAsSource -AutoSize
Get-NetUDPEndpoint -ErrorAction SilentlyContinue |
    Where-Object { $_.LocalAddress -eq $IcsPrivateIp -and $_.LocalPort -in 67,68 } |
    Sort-Object LocalPort |
    Format-Table LocalAddress,LocalPort,OwningProcess -AutoSize

if (-not $SkipTargetCheck) {
    Get-NetNeighbor -InterfaceAlias $PrivateInterface -IPAddress $TargetIp -ErrorAction SilentlyContinue |
        Format-Table IPAddress,LinkLayerAddress,State -AutoSize
    Write-Host "BRDP TCP $TargetIp`:$TargetPort depuis $LabSourceIp : OK" -ForegroundColor Green
}
Write-Host ".env : BOUCHAUD_LAB_SOURCE_IP enregistre (aucun secret affiche)." -ForegroundColor Green
Write-Host "LAB_HOST_READY_V4" -ForegroundColor Green
