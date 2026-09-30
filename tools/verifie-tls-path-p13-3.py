#!/usr/bin/env python3
"""Garde-fou P13.3 : le chemin TLS s'arrete a une ancre locale, et seulement la.

# Ce que P13.3 a corrige

example.com envoie une chaine dont le dernier certificat est une racine
SSL.com CROSS-SIGNEE par une AC absente du magasin. L'ancien parcours exigeait
que le DERNIER certificat envoye soit signe par une racine locale : la chaine
etait refusee alors qu'un de ses maillons l'etait deja.

La correction (`BOUCHAUD_P13_3_PATH_PREFIX_V2`) remonte depuis la feuille et
s'arrete DES qu'un maillon est une ancre locale (meme sujet et meme cle
publique qu'une racine du magasin) ou est signe par une racine du magasin. Ce
qui suit n'appartient pas au chemin et ne peut plus le faire echouer.

# Pourquoi cette garde a ete reecrite

Elle a ete ajoutee dans le MEME commit que la correction (4de41fde), mais
ecrite contre un brouillon anterieur (`..._CROSS_SIGNED_ANCHOR_V1`, qui
validait toute la chaine puis cherchait l'ancre sur `last`). Elle n'a donc
jamais pu passer sur aucun commit, et son echec faisait sauter les etapes
suivantes de CI Fast et les jobs QEMU de Reliability V3.

Chacune de ses verifications V1 a ete relue contre le code V2 : la propriete
qu'elle protegeait y est toujours, sous un autre nom. Elles sont reecrites
ici sur ce nom, et durcies par des verifications d'ORDRE que la version
textuelle ne faisait pas.
"""
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
ROOTS = ROOT / "src" / "net" / "security" / "tls" / "roots.rs"
VALIDATE = ROOT / "src" / "net" / "security" / "tls" / "validate.rs"


def fail(msg: str) -> None:
    print(f"P13_3_TLS_PATH_FAIL: {msg}", file=sys.stderr)
    raise SystemExit(1)


def sans_commentaires(texte: str) -> str:
    """Le code seul : un commentaire qui CITE une regle ne la fait pas exister."""
    return "\n".join(re.sub(r"//.*$", "", ligne) for ligne in texte.splitlines())


def bloc(source: str, entete: str):
    """Le corps `{ ... }` qui suit `entete`, ou None."""
    debut = source.find(entete)
    if debut < 0:
        return None
    i = source.find("{", debut)
    if i < 0:
        return None
    profondeur = 0
    for j in range(i, len(source)):
        if source[j] == "{":
            profondeur += 1
        elif source[j] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[i:j + 1]
    return None


roots = ROOTS.read_text(encoding="utf-8")
validate_brut = VALIDATE.read_text(encoding="utf-8")
validate = sans_commentaires(validate_brut)

# --- roots.rs : l'ancre equivalente est une egalite STRICTE ----------------
checks = {
    "roots marker": "BOUCHAUD_P13_3_EQUIVALENT_TRUST_ANCHOR_V1" in roots,
    "equivalent anchor fn": "pub fn find_equivalent_anchor(cert: &Certificate)" in roots,
    "subject equality": "root.subject == cert.subject" in roots,
    "SPKI/public key equality": "meme_cle_publique(&root.pubkey, &cert.pubkey)" in roots,
    "RSA key equality": "an == bn && ae == be" in roots,
    "P256 key equality": "PubKey::EcP256" in roots and "ap == bp" in roots,
    "P384 key equality": "PubKey::EcP384" in roots,
    # L'emetteur local n'est retenu que s'il SIGNE : le sujet seul se recopie.
    "issuer signature checked": "x509::verify_signed_by(child, &root.pubkey)" in roots,
}
for name, ok in checks.items():
    if not ok:
        fail(name)

# --- validate.rs : le parcours V2 ------------------------------------------
if "BOUCHAUD_P13_3_PATH_PREFIX_V2" not in validate_brut:
    fail("validate marker")

ancre_equivalente = "if roots::find_equivalent_anchor(courant).is_some()"
ancre_emettrice = "if roots::find_issuer_for(courant).is_some()"
suivant = "certs.get(i + 1)"
coherence = "courant.issuer != suivant.subject"
signature = "x509::verify_signed_by(courant, &suivant.pubkey)"

for nom, jeton in (
    ("equivalent anchor used", ancre_equivalente),
    ("legacy issuer path preserved", ancre_emettrice),
    ("next certificate required", suivant),
    ("issuer subject hardening", coherence),
    ("chain signature verified", signature),
):
    if jeton not in validate:
        fail(nom)

# LA CONFIANCE NE NAIT QUE D'UNE ANCRE. Deux affectations, chacune dans une
# branche d'ancre ; une troisieme ferait confiance a autre chose.
if validate.count("res.trusted = true") != 2:
    fail("trusted only after anchor: res.trusted = true hors des deux branches d'ancre")
for nom, entete in (("equivalente", ancre_equivalente), ("emettrice", ancre_emettrice)):
    corps = bloc(validate, entete)
    if corps is None or "res.trusted = true" not in corps or "return res" not in corps:
        fail(f"trusted only after anchor: branche d'ancre {nom} incomplete")
if 'res.anchor = Some("magasin:subject+spki")' not in validate:
    fail("trusted only after anchor: ancre equivalente non nommee")

# L'ORDRE EST LA REGLE. On ne regarde le certificat suivant qu'apres avoir
# constate que le courant n'est pas deja une ancre, et on ne verifie une
# signature qu'entre deux maillons qui se designent.
positions = [validate.find(j) for j in (ancre_equivalente, ancre_emettrice, suivant, coherence, signature)]
if positions != sorted(positions):
    fail("ordre du parcours : ancre equivalente, ancre emettrice, suivant, "
         "coherence issuer/subject, signature")

# Le patch ne doit pas faire confiance a AAA par ajout opportuniste au magasin.
if 'include_bytes!("ca/AAA' in roots or 'AAA_Certificate' in roots:
    fail("AAA ne doit pas etre ajoutee comme trust anchor par P13.3")

print("P13_3_TLS_PATH_VERIFY_OK")
