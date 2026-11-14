#!/usr/bin/env python3
"""BOUCHAUD_E2E_LADYBIRD_V1 -- materiel TLS de BANC, genere AU MOMENT du test.

    python3 genere_tls.py DOSSIER

Produit dans DOSSIER (jamais dans le depot : tools/verifie-cle-dev.py
refuse toute cle privee commitee) :
  ca.pem          autorite de TEST, valide a partir de maintenant ;
  expire.pem      certificat pour l'IP 10.0.2.2 (l'hote vu de l'invite QEMU),
                  signe par cette autorite, EXPIRE depuis le 02/01/2025 ;
  expire.key      sa cle privee (et ca.key, celle de l'autorite).

Un banc qui prouve qu'un certificat expire est refuse a besoin d'une
autorite que le navigateur CROIT : sinon le refus viendrait de l'autorite
inconnue, pas de l'expiration. ca.pem n'est ajoute qu'au disque des
parcours E2E (prepare_disque.py), jamais a une image livree.

Deux moyens, dans l'ordre : la bibliotheque `cryptography`, puis la CLI
`openssl` (`openssl ca`, seule a savoir dater un certificat dans le passe).
Sans l'un ni l'autre : code 3, et le parcours qui en depend est
NON_MESURABLE -- jamais PASS.
"""
import datetime
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

DEBUT = "20250101000000Z"
FIN = "20250102000000Z"


def par_cryptography(d: Path) -> bool:
    try:
        import ipaddress
        from cryptography import x509
        from cryptography.hazmat.primitives import hashes, serialization
        from cryptography.hazmat.primitives.asymmetric import ec
        from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID
    except ImportError:
        return False
    utc = datetime.timezone.utc

    def nom(cn):
        return x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, cn),
                          x509.NameAttribute(NameOID.ORGANIZATION_NAME, "Bouchaud OS banc E2E")])

    cle_ca = ec.generate_private_key(ec.SECP256R1())
    maintenant = datetime.datetime.now(utc)
    ca = (x509.CertificateBuilder()
          .subject_name(nom("Bouchaud E2E CA de test")).issuer_name(nom("Bouchaud E2E CA de test"))
          .public_key(cle_ca.public_key()).serial_number(x509.random_serial_number())
          .not_valid_before(maintenant - datetime.timedelta(days=1))
          .not_valid_after(maintenant + datetime.timedelta(days=30))
          .add_extension(x509.BasicConstraints(ca=True, path_length=0), critical=True)
          .add_extension(x509.KeyUsage(digital_signature=True, key_cert_sign=True, crl_sign=True,
                                       content_commitment=False, key_encipherment=False,
                                       data_encipherment=False, key_agreement=False,
                                       encipher_only=False, decipher_only=False), critical=True)
          .add_extension(x509.SubjectKeyIdentifier.from_public_key(cle_ca.public_key()), critical=False)
          .sign(cle_ca, hashes.SHA256()))
    cle = ec.generate_private_key(ec.SECP256R1())
    feuille = (x509.CertificateBuilder()
               .subject_name(nom("10.0.2.2")).issuer_name(ca.subject)
               .public_key(cle.public_key()).serial_number(x509.random_serial_number())
               .not_valid_before(datetime.datetime(2025, 1, 1, tzinfo=utc))
               .not_valid_after(datetime.datetime(2025, 1, 2, tzinfo=utc))
               .add_extension(x509.SubjectAlternativeName([x509.IPAddress(ipaddress.ip_address("10.0.2.2"))]),
                              critical=False)
               .add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.SERVER_AUTH]), critical=False)
               # Les verificateurs stricts (X509_STRICT) exigent ces deux
               # identifiants : sans eux, le refus viendrait de leur absence.
               .add_extension(x509.SubjectKeyIdentifier.from_public_key(cle.public_key()), critical=False)
               .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(cle_ca.public_key()),
                              critical=False)
               .sign(cle_ca, hashes.SHA256()))
    pem = serialization.Encoding.PEM
    nue = (serialization.PrivateFormat.PKCS8, serialization.NoEncryption())
    (d / "ca.pem").write_bytes(ca.public_bytes(pem))
    (d / "ca.key").write_bytes(cle_ca.private_bytes(pem, *nue))
    (d / "expire.pem").write_bytes(feuille.public_bytes(pem))
    (d / "expire.key").write_bytes(cle.private_bytes(pem, *nue))
    return True


CONFIG = """[ ca ]
default_ca = banc
[ banc ]
dir = {dir}
database = $dir/index.txt
serial = $dir/serial
new_certs_dir = $dir
certificate = $dir/ca.pem
private_key = $dir/ca.key
default_md = sha256
policy = libre
x509_extensions = feuille
copy_extensions = none
unique_subject = no
[ libre ]
commonName = supplied
[ feuille ]
subjectAltName = IP:10.0.2.2
extendedKeyUsage = serverAuth
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid
"""


def par_openssl(d: Path) -> bool:
    openssl = shutil.which("openssl")
    if not openssl:
        return False
    with tempfile.TemporaryDirectory() as t:
        t = Path(t)
        (t / "index.txt").write_text("")
        (t / "serial").write_text("1000\n")
        (t / "banc.cnf").write_text(CONFIG.format(dir=t.as_posix()))
        cle = ["-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes"]
        etapes = [
            [openssl, "req", "-x509", *cle, "-keyout", str(t / "ca.key"), "-out", str(t / "ca.pem"),
             "-days", "30", "-subj", "/CN=Bouchaud E2E CA de test/O=Bouchaud OS banc E2E",
             "-addext", "basicConstraints=critical,CA:TRUE,pathlen:0",
             "-addext", "keyUsage=critical,keyCertSign,cRLSign",
             "-addext", "subjectKeyIdentifier=hash"],
            [openssl, "req", "-new", *cle, "-keyout", str(t / "expire.key"), "-out", str(t / "expire.csr"),
             "-subj", "/CN=10.0.2.2"],
            [openssl, "ca", "-batch", "-config", str(t / "banc.cnf"), "-in", str(t / "expire.csr"),
             "-out", str(t / "expire.pem"), "-startdate", DEBUT, "-enddate", FIN, "-notext"],
        ]
        for e in etapes:
            if subprocess.run(e, capture_output=True).returncode != 0:
                return False
        for nom in ("ca.pem", "ca.key", "expire.pem", "expire.key"):
            shutil.copyfile(t / nom, d / nom)
    return True


def genere(d: Path) -> str | None:
    """Genere le materiel dans `d` ; rend le moyen employe, ou None."""
    d.mkdir(parents=True, exist_ok=True)
    if par_cryptography(d):
        return "cryptography"
    if par_openssl(d):
        return "openssl"
    return None


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        sys.exit(2)
    moyen = genere(Path(sys.argv[1]))
    if not moyen:
        print("E2E_TLS_INDISPONIBLE ni cryptography ni openssl", file=sys.stderr)
        sys.exit(3)
    print(f"E2E_TLS_PRET dossier={sys.argv[1]} moyen={moyen}")
