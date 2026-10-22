# BOUCHAUD_QEMU_WINDOWS_SAFE_V1
# Shared QEMU Windows helpers. No guest disk is writable by default.
Set-StrictMode -Version Latest

function Get-BouchaudQemuExe {
    $candidates = @(
        'C:\Program Files\qemu\qemu-system-x86_64.exe',
        'C:\Program Files (x86)\qemu\qemu-system-x86_64.exe'
    )
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
    }
    $cmd = Get-Command 'qemu-system-x86_64.exe' -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    throw 'QEMU Windows introuvable. Installe QEMU Windows ou ajoute-le au PATH.'
}

function Get-BouchaudOvmf {
    param([Parameter(Mandatory = $true)][string]$QemuExe)
    $share = Join-Path (Split-Path -Parent $QemuExe) 'share'
    $codeCandidates = @(
        (Join-Path $share 'edk2-x86_64-code.fd'),
        (Join-Path $share 'OVMF_CODE.fd')
    )
    $varsCandidates = @(
        (Join-Path $share 'edk2-i386-vars.fd'),
        (Join-Path $share 'edk2-x86_64-vars.fd'),
        (Join-Path $share 'OVMF_VARS.fd')
    )
    $code = $codeCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    $vars = $varsCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    if (-not $code -or -not $vars) {
        throw "OVMF UEFI introuvable dans $share (CODE et VARS requis)."
    }
    return @{ Code = [string]$code; Vars = [string]$vars }
}

function Get-BouchaudDisplayArguments {
    param(
        [Parameter(Mandatory = $true)][string]$QemuExe,
        [ValidateSet('auto','gtk','sdl','none')][string]$Display = 'auto',
        [switch]$Fullscreen
    )
    # QEMU prints its backends on stdout on some versions, stderr on others.
    $output = (& $QemuExe -display help 2>&1 | Out-String)
    $backend = $Display
    if ($backend -eq 'auto') {
        if ($output -match '(?m)^\s*gtk\s*$') { $backend = 'gtk' }
        elseif ($output -match '(?m)^\s*sdl\s*$') { $backend = 'sdl' }
        else { throw 'Aucun backend GTK/SDL : indique -Display none pour test sans fenetre.' }
    }
    if (($backend -ne 'none') -and ($output -notmatch "(?m)^\s*$backend\s*$")) {
        throw "Backend QEMU '$backend' indisponible (qemu-system-x86_64 -display help)."
    }
    switch ($backend) {
        'gtk'  { $displayArgs = @('-display','gtk,zoom-to-fit=on,gl=off') }
        'sdl'  { $displayArgs = @('-display','sdl,gl=off') }
        'none' { $displayArgs = @('-display','none') }
    }
    if ($Fullscreen -and $backend -ne 'none') { $displayArgs += '-full-screen' }
    Write-Host "QEMU_DISPLAY_BACKEND=$backend"
    if ($backend -eq 'sdl') {
        Write-Host 'NOTE : SDL ne garantit pas le zoom GTK. Utiliser -Display gtk si disponible.' -ForegroundColor Yellow
    }
    return $displayArgs
}

function Get-BouchaudInputArguments {
    param(
        [ValidateSet('usb','ps2','none')][string]$InputMode = 'usb',
        [ValidateSet('mouse','tablet')][string]$Pointer = 'mouse'
    )
    if ($InputMode -eq 'usb') {
        return @(
            '-device','qemu-xhci,id=xhci',
            '-device','usb-kbd,bus=xhci.0',
            '-device',"usb-$Pointer,bus=xhci.0"
        )
    }
    if ($InputMode -eq 'ps2') {
        Write-Host 'QEMU_INPUT=ps2 : peripheriques PS/2 de la machine pc, sans xHCI.' -ForegroundColor Yellow
    }
    return @()
}

function New-BouchaudQemuSerialPath {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [string]$Profile = 'trigkey',
        [switch]$DryRun
    )
    $dir = Join-Path $RepoRoot 'target\reference\qemu-logs'
    if (-not $DryRun) {
        New-Item -Path $dir -ItemType Directory -Force | Out-Null
    }
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
    $name = ('{0}-{1}-{2}.log' -f $Profile, $stamp, $PID)
    return (Join-Path $dir $name)
}

function Invoke-BouchaudQemu {
    param(
        [Parameter(Mandatory = $true)][string]$QemuExe,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$SerialPath,
        [switch]$DryRun
    )
    $writable = @($Arguments | Where-Object {
        $_ -match '^if=ide,index=1,media=disk,format=raw,file=' -and $_ -notmatch 'snapshot=on'
    }).Count -gt 0
    if ($writable) {
        Write-Host 'QEMU_BOOT_SNAPSHOT=ON; PERSIST_LAB_DISK=WRITABLE (EXPLICIT OPT-IN)' -ForegroundColor Yellow
    } else {
        Write-Host 'QEMU_SNAPSHOT=ON (aucun disque de test inscriptible)' -ForegroundColor Green
    }
    Write-Host "QEMU_SERIAL=$SerialPath"
    if ($DryRun) {
        Write-Host 'QEMU_DRY_RUN_OK: NOT_STARTED; NO_IMAGE_OR_DISK_WRITE' -ForegroundColor Green
        for ($i = 0; $i -lt $Arguments.Length; $i++) {
            Write-Host ('  [{0}] {1}' -f $i, $Arguments[$i])
        }
        return 0
    }
    & $QemuExe @Arguments
    return $LASTEXITCODE
}
