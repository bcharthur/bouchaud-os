<#
.SYNOPSIS
    Fabrique le magasin d'autorites que RequestServer passe a curl.

.DESCRIPTION
    Ladybird ne verifie une chaine TLS que s'il recoit un chemin de bundle :
    sans `--certificate`, curl retombe sur le chemin compile a la construction,
    qui n'existe pas dans Bouchaud OS. Le fichier produit ici est copie par
    `run.ps1` dans le disque de scenario, en /etc/ssl/certs/ca-certificates.crt.

    Deux sources, dans cet ordre :

      1. le magasin racine de Windows - c'est-a-dire exactement les autorites
         auxquelles cette machine fait deja confiance. Rien n'est telecharge,
         et aucune decision de confiance nouvelle n'est prise en votre nom.
      2. a defaut, les racines DER deja versionnees dans
         `src/net/security/tls/ca/`, celles de la pile TLS du noyau.

    La verification n'est jamais relachee. Un bundle vide fait echouer le
    script plutot que produire un fichier qui ferait echouer chaque poignee de
    main TLS sans dire pourquoi.

.EXAMPLE
    .\tools\ladybird\certs\fabrique-bundle.ps1
#>

[CmdletBinding()]
param(
    # Chemin du bundle produit.
    [string]$Sortie,

    # Reconstruit meme si le fichier existe deja.
    [switch]$Force
)

$ErrorActionPreference = "Stop"

# Le script vit dans <depot>/tools/ladybird/certs/ : quatre remontees, pas
# trois. Avec trois, la racine tombait sur <depot>/tools, le bundle etait ecrit
# dans tools/tools/ladybird/certs/ (donc hors du chemin ignore par git et hors
# de celui que run.ps1 lit), et le repli DER cherchait tools/src/net/... pour
# annoncer qu'aucune racine n'existe.
$RepoRoot = Split-Path -Parent (
    Split-Path -Parent (
        Split-Path -Parent (
            Split-Path -Parent $PSCommandPath
        )
    )
)

if (-not $Sortie) {
    $Sortie = Join-Path $RepoRoot "tools\ladybird\certs\cacert.pem"
}

if ((Test-Path -LiteralPath $Sortie -PathType Leaf) -and -not $Force) {
    Write-Host "bundle deja present : $Sortie" -ForegroundColor DarkGray
    return $Sortie
}


function ConvertTo-Pem {
    param(
        [Parameter(Mandatory = $true)]
        [System.Security.Cryptography.X509Certificates.X509Certificate2]$Certificat
    )

    $base64 = [System.Convert]::ToBase64String(
        $Certificat.RawData,
        [System.Base64FormattingOptions]::InsertLineBreaks
    )

    return (
        "# " + $Certificat.Subject + "`n" +
        "-----BEGIN CERTIFICATE-----`n" +
        $base64.Replace("`r`n", "`n").TrimEnd() + "`n" +
        "-----END CERTIFICATE-----`n"
    )
}


$blocs = New-Object System.Collections.Generic.List[string]
$empreintes = New-Object System.Collections.Generic.HashSet[string]
$source = "aucune"


# --- 1. Magasin racine du systeme -------------------------------------------

try {
    foreach ($emplacement in @("LocalMachine", "CurrentUser")) {
        $magasin = New-Object System.Security.Cryptography.X509Certificates.X509Store(
            [System.Security.Cryptography.X509Certificates.StoreName]::Root,
            $emplacement
        )
        $magasin.Open(
            [System.Security.Cryptography.X509Certificates.OpenFlags]::ReadOnly
        )

        foreach ($certificat in $magasin.Certificates) {
            # Un certificat expire dans le magasin ferait echouer une chaine
            # valide en la faisant aboutir sur une racine perimee.
            if ($certificat.NotAfter -lt (Get-Date)) {
                continue
            }
            if (-not $empreintes.Add($certificat.Thumbprint)) {
                continue
            }
            $blocs.Add((ConvertTo-Pem -Certificat $certificat))
        }

        $magasin.Close()
    }

    if ($blocs.Count -gt 0) {
        $source = "magasin racine du systeme"
    }
}
catch {
    Write-Host (
        "magasin Windows inaccessible ({0}), repli sur les racines du depot" -f `
            $_.Exception.Message
    ) -ForegroundColor Yellow
}


# --- 2. Repli : les racines DER de la pile TLS du noyau ----------------------

if ($blocs.Count -eq 0) {
    $racines = Join-Path $RepoRoot "src\net\security\tls\ca"

    if (Test-Path -LiteralPath $racines -PathType Container) {
        foreach ($fichier in Get-ChildItem -LiteralPath $racines -Filter *.der) {
            $certificat = New-Object `
                System.Security.Cryptography.X509Certificates.X509Certificate2(
                    $fichier.FullName
                )
            if (-not $empreintes.Add($certificat.Thumbprint)) {
                continue
            }
            $blocs.Add((ConvertTo-Pem -Certificat $certificat))
        }
    }

    if ($blocs.Count -gt 0) {
        $source = "racines DER du depot (src/net/security/tls/ca)"
    }
}


if ($blocs.Count -eq 0) {
    throw "aucune autorite racine trouvee : ni magasin Windows, ni racines du depot"
}


$dossier = Split-Path -Parent $Sortie
if (-not (Test-Path -LiteralPath $dossier -PathType Container)) {
    New-Item -ItemType Directory -Path $dossier -Force | Out-Null
}

$entete = (
    "# Magasin d'autorites pour RequestServer (Bouchaud OS, jalon M12).`n" +
    "# Source : $source`n" +
    "# Genere le " + (Get-Date -Format "yyyy-MM-dd HH:mm:ss") + " par " +
    "tools/ladybird/certs/fabrique-bundle.ps1`n" +
    "# Ne pas versionner : ce fichier decrit la confiance de CETTE machine.`n"
)

[System.IO.File]::WriteAllText(
    $Sortie,
    $entete + ($blocs -join ""),
    [System.Text.UTF8Encoding]::new($false)
)

Write-Host (
    "bundle CA : {0} autorites depuis {1}" -f $blocs.Count, $source
) -ForegroundColor Green
Write-Host "  -> $Sortie" -ForegroundColor DarkGray

return $Sortie
