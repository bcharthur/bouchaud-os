#!/usr/bin/env python3
"""BOUCHAUD_E2E_LADYBIRD_V1 -- materiel TLS de BANC, regenere a la demande.

    python3 tools/e2e/ladybird/tls/genere_tls.py

Produit, dans ce dossier :
  ca.pem          autorite de TEST, valide 2026 -> 2046 ;
  expire.pem      certificat pour l'IP 10.0.2.2 (l'hote vu de l'invite QEMU),
                  signe par cette autorite, EXPIRE depuis le 02/01/2025 ;
  expire.key      sa cle privee.

Ce ne sont PAS des secrets : un banc qui prouve qu'un certificat expire est
refuse a besoin d'une autorite que le navigateur CROIT (sinon le refus
viendrait de l'autorite inconnue, pas de l'expiration). `ca.pem` n'est
ajoute qu'au disque des parcours E2E (prepare_disque.py), jamais a une
image livree. Les fichiers sont commites pour que le banc tourne partout,
Windows compris, sans openssl ni `cryptography`.
"""
import datetime
import ipaddress
from pathlib import Path

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import NameOID

ICI = Path(__file__).resolve().parent
UTC = datetime.timezone.utc


def nom(cn: str) -> x509.Name:
    return x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, cn),
                      x509.NameAttribute(NameOID.ORGANIZATION_NAME, "Bouchaud OS banc E2E")])


def main() -> None:
    cle_ca = ec.generate_private_key(ec.SECP256R1())
    ca = (x509.CertificateBuilder()
          .subject_name(nom("Bouchaud E2E CA de test")).issuer_name(nom("Bouchaud E2E CA de test"))
          .public_key(cle_ca.public_key()).serial_number(x509.random_serial_number())
          .not_valid_before(datetime.datetime(2026, 1, 1, tzinfo=UTC))
          .not_valid_after(datetime.datetime(2046, 1, 1, tzinfo=UTC))
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
               .not_valid_before(datetime.datetime(2025, 1, 1, tzinfo=UTC))
               .not_valid_after(datetime.datetime(2025, 1, 2, tzinfo=UTC))
               .add_extension(x509.SubjectAlternativeName([x509.IPAddress(ipaddress.ip_address("10.0.2.2"))]),
                              critical=False)
               .add_extension(x509.ExtendedKeyUsage([x509.oid.ExtendedKeyUsageOID.SERVER_AUTH]), critical=False)
               # Les verificateurs stricts (X509_STRICT) exigent ces deux
               # identifiants : sans eux, le refus viendrait de leur absence,
               # pas de l'expiration (test RefusTls).
               .add_extension(x509.SubjectKeyIdentifier.from_public_key(cle.public_key()), critical=False)
               .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(cle_ca.public_key()), critical=False)
               .sign(cle_ca, hashes.SHA256()))
    (ICI / "ca.pem").write_bytes(ca.public_bytes(serialization.Encoding.PEM))
    (ICI / "expire.pem").write_bytes(feuille.public_bytes(serialization.Encoding.PEM))
    (ICI / "expire.key").write_bytes(cle.private_bytes(serialization.Encoding.PEM,
                                                       serialization.PrivateFormat.PKCS8,
                                                       serialization.NoEncryption()))


if __name__ == "__main__":
    main()
