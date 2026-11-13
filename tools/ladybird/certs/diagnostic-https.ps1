<#
.SYNOPSIS
    Pourquoi Ladybird refuse un site HTTPS (curl `code=60`, `verify=N`) ?

.DESCRIPTION
    BOUCHAUD_CA_COMPLET_V2 -- un diagnostic, cote HOTE Windows, de ce que voit
    RequestServer sous QEMU (le trafic de l'invite sort par la pile reseau de
    l'hote : SLIRP). Il ne relache RIEN et ne modifie rien :

      1. la chaine REELLE que le serveur presente a cette machine (sujet,
         emetteur, validite, empreinte de chaque maillon) ;
      2. la racine de cette chaine est-elle dans le bundle de Ladybird
         (tools/ladybird/certs/cacert.pem) ? Sinon : `verify=20` ;
      3. interception TLS par l'hote (antivirus, proxy d'entreprise) : la
         racine n'est alors PAS une racine publique du programme Microsoft
         (Windows Update) -- le script le dit, et nomme l'emetteur ;
      4. l'horloge : un ecart entre l'heure de l'hote et la validite des
         certificats donnerait `verify=9/10` (pas encore valide / expire).

    Code de sortie : 0 tout est coherent ; 1 racine absente du bundle ;
    2 interception probable ; 3 connexion impossible.

    Non qualifie sur Windows par la session qui l'a ecrit (aucun Windows
    disponible) : la CI l'execute sous pwsh Linux (refus et syntaxe).

.EXAMPLE
    .\tools\ladybird\certs\diagnostic-https.ps1 -Hote example.com
#>
[CmdletBinding()]
param(
    [string]$Hote = "example.com",
    [int]$Port = 443,
    [string]$Bundle
)
$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSCommandPath)))
if (-not $Bundle) { $Bundle = Join-Path $RepoRoot "tools\ladybird\certs\cacert.pem" }

Write-Host ("heure hote (UTC) : {0:yyyy-MM-dd HH:mm:ss}" -f (Get-Date).ToUniversalTime())

$chaine = New-Object System.Collections.Generic.List[System.Security.Cryptography.X509Certificates.X509Certificate2]
try {
    $tcp = New-Object System.Net.Sockets.TcpClient($Hote, $Port)
    $rappel = [System.Net.Security.RemoteCertificateValidationCallback] {
        param($expediteur, $certificat, $ch, $erreurs)
        foreach ($e in $ch.ChainElements) { $chaine.Add([System.Security.Cryptography.X509Certificates.X509Certificate2]::new($e.Certificate)) }
        $script:ErreursPolitique = $erreurs
        return $true   # on OBSERVE la chaine ; rien n'est envoye au serveur apres la poignee
    }
    $ssl = New-Object System.Net.Security.SslStream($tcp.GetStream(), $false, $rappel)
    $ssl.AuthenticateAsClient($Hote)
    $ssl.Dispose(); $tcp.Dispose()
}
catch {
    Write-Host ("HTTPS_DIAG connexion_impossible hote={0}:{1} raison={2}" -f $Hote, $Port, $_.Exception.Message) -ForegroundColor Red
    exit 3
}

$i = 0
foreach ($c in $chaine) {
    Write-Host ("  [{0}] sujet={1}" -f $i, $c.Subject)
    Write-Host ("      emetteur={0} valide={1:yyyy-MM-dd}..{2:yyyy-MM-dd} sha1={3}" -f $c.Issuer, $c.NotBefore, $c.NotAfter, $c.Thumbprint)
    $i++
}
$racine = $chaine[$chaine.Count - 1]
Write-Host ("politique Windows : {0}" -f $script:ErreursPolitique)

# 2. La racine est-elle dans le bundle ?
$dansBundle = $false
if (Test-Path -LiteralPath $Bundle) {
    $texte = Get-Content -LiteralPath $Bundle -Raw
    foreach ($m in [regex]::Matches($texte, "-----BEGIN CERTIFICATE-----[\s\S]+?-----END CERTIFICATE-----")) {
        $der = [System.Convert]::FromBase64String(($m.Value -replace "-----(BEGIN|END) CERTIFICATE-----", "" -replace "\s", ""))
        $x = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2(, $der)
        if ($x.Thumbprint -eq $racine.Thumbprint) { $dansBundle = $true; break }
    }
    Write-Host ("bundle : {0} ({1})" -f $Bundle, $(if ($texte -match "BOUCHAUD_CA_COMPLET_V2") { "schema V2" } else { "ANCIEN schema : magasin local seul, incomplet" }))
}
else {
    Write-Host "bundle absent : $Bundle" -ForegroundColor Yellow
}

# 3. Racine publique (programme Microsoft) ou interception ?
$publique = $null
$certutil = Get-Command certutil.exe -ErrorAction SilentlyContinue
if ($certutil) {
    $sst = Join-Path ([System.IO.Path]::GetTempPath()) ("bouchaud-diag-" + [guid]::NewGuid() + ".sst")
    try {
        & $certutil.Source -generateSSTFromWU $sst | Out-Null
        if ($LASTEXITCODE -eq 0) {
            $col = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2Collection
            $col.Import($sst)
            $publique = [bool]($col | Where-Object { $_.Thumbprint -eq $racine.Thumbprint })
        }
    }
    finally { Remove-Item -LiteralPath $sst -Force -ErrorAction SilentlyContinue }
}

if ($publique -eq $false) {
    Write-Host ("HTTPS_DIAG interception_probable racine=""{0}"" : ce n'est pas une racine publique ; un antivirus ou un proxy de l'hote dechiffre le trafic. Ladybird a raison de refuser." -f $racine.Subject) -ForegroundColor Red
    exit 2
}
if (-not $dansBundle) {
    Write-Host ("HTTPS_DIAG racine_absente_du_bundle racine=""{0}"" : curl repond verify=20. Reconstruire : tools\ladybird\certs\fabrique-bundle.ps1 -Force" -f $racine.Subject) -ForegroundColor Yellow
    exit 1
}
Write-Host ("HTTPS_DIAG coherent racine=""{0}"" dans_bundle=oui publique={1}" -f $racine.Subject, $(if ($null -eq $publique) { "inconnu(certutil absent)" } else { "oui" })) -ForegroundColor Green
exit 0
