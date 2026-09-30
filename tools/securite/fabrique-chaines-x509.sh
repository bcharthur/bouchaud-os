#!/usr/bin/env bash
# Fabrique les chaines X.509 de `test_chaine_x509.rs`.
#
# POURQUOI DES FICHIERS, ET PAS UNE GENERATION AU MOMENT DU TEST
#
# Le test est compile par `rustc --test`, sans caisse externe : il ne sait pas
# SIGNER. Il ne sait que verifier -- avec le code du noyau, ce qui est tout
# l'interet. Les chaines sont donc produites ici, une fois, par une
# implementation qui ne partage AUCUNE ligne avec `x509.rs` (OpenSSL), puis
# versionnees.
#
# Relancer ce script produit des octets differents -- ECDSA tire un alea par
# signature, les cles sont neuves -- mais des chaines de meme structure : le
# test ne depend que de la structure. Le relancer n'est utile que pour AJOUTER
# une chaine.
#
#     tools/securite/fabrique-chaines-x509.sh
#
# CE QUE CONTIENNENT LES CHAINES
#
# Une racine de test, jamais installee ailleurs que dans le magasin factice du
# test, et sous elle :
#
# - la chaine honnete : feuille `victime.test` <- intermediaire CA:TRUE ;
# - la chaine de l'attaquant : il detient une feuille HONNETE pour
#   `attaquant.test` (CA:FALSE, emise normalement par l'intermediaire) et s'en
#   sert pour signer une fausse feuille `victime.test`. C'est l'attaque
#   basicConstraints classique : ce n'est pas la signature qui autorise un
#   maillon, c'est le DRAPEAU CA de son signataire ;
# - un intermediaire v3 SANS extension basicConstraints : RFC 5280 4.2.1.9,
#   cA vaut alors FALSE, il ne peut rien signer ;
# - une feuille signee directement par la racine ;
# - une chaine vers une racine inconnue du magasin.
#
# Toutes les cles sont ECDSA P-256 / SHA-256 : c'est la courbe que le noyau
# verifie le plus vite, et le sujet ici est la construction du chemin, pas
# l'algorithme.
#
# La validite part de l'instant de fabrication et dure 29 000 jours. Le test
# passe son `now` explicitement ; l'heure de la machine qui lance la suite
# n'intervient jamais.
set -euo pipefail
cd "$(dirname "$0")"

SORTIE=chaines-x509
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$SORTIE"

JOURS=29000
ORG="/O=Bouchaud OS test"

# Aucune extension par defaut : chacune est dite ici, sinon le cas « sans
# basicConstraints » pourrait en recevoir une de openssl.cnf.
cat > "$TMP/vide.cnf" <<'EOF'
[ req ]
distinguished_name = dn
[ dn ]
EOF

ext_ca() { printf 'basicConstraints=critical,CA:TRUE\nsubjectKeyIdentifier=hash\n' > "$TMP/$1.ext"; }
ext_sans_bc() { printf 'subjectKeyIdentifier=hash\n' > "$TMP/$1.ext"; }
ext_feuille() {
    printf 'basicConstraints=critical,CA:FALSE\nsubjectKeyIdentifier=hash\nsubjectAltName=DNS:%s\n' \
        "$2" > "$TMP/$1.ext"
}

cle() {
    openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$TMP/$1.key" 2>/dev/null
}

# racine auto-signee : nom, CN
racine() {
    cle "$1"
    ext_ca "$1"
    openssl req -config "$TMP/vide.cnf" -new -key "$TMP/$1.key" -subj "$ORG/CN=$2" \
        -out "$TMP/$1.csr"
    openssl x509 -req -in "$TMP/$1.csr" -key "$TMP/$1.key" -days "$JOURS" -sha256 \
        -set_serial "0x$(openssl rand -hex 8)" -extfile "$TMP/$1.ext" \
        -out "$TMP/$1.pem" 2>/dev/null
}

# certificat signe : nom, CN, emetteur (nom deja fabrique), fichier d'extensions
signe() {
    cle "$1"
    openssl req -config "$TMP/vide.cnf" -new -key "$TMP/$1.key" -subj "$ORG/CN=$2" \
        -out "$TMP/$1.csr"
    openssl x509 -req -in "$TMP/$1.csr" -CA "$TMP/$3.pem" -CAkey "$TMP/$3.key" \
        -days "$JOURS" -sha256 -set_serial "0x$(openssl rand -hex 8)" \
        -extfile "$TMP/$1.ext" -out "$TMP/$1.pem" 2>/dev/null
}

racine racine "Bouchaud Test Racine"

ext_ca intermediaire
signe intermediaire "Bouchaud Test Intermediaire" racine

ext_feuille feuille victime.test
signe feuille "victime.test" intermediaire

# L'attaquant : une feuille HONNETE, qu'il a obtenue normalement...
ext_feuille feuille-attaquant attaquant.test
signe feuille-attaquant "attaquant.test" intermediaire
# ... dont il se sert comme d'une AC. OpenSSL signe sans regarder le drapeau
# du signataire : c'est au verificateur de le faire.
ext_feuille feuille-forgee victime.test
signe feuille-forgee "victime.test" feuille-attaquant

ext_sans_bc intermediaire-sans-bc
signe intermediaire-sans-bc "Bouchaud Test Intermediaire Sans BC" racine
ext_feuille feuille-sous-sans-bc victime.test
signe feuille-sous-sans-bc "victime.test" intermediaire-sans-bc

ext_feuille feuille-directe direct.test
signe feuille-directe "direct.test" racine

racine racine-inconnue "Bouchaud Test Racine Inconnue"
ext_feuille feuille-inconnue victime.test
signe feuille-inconnue "victime.test" racine-inconnue

for nom in racine intermediaire feuille feuille-attaquant feuille-forgee \
           intermediaire-sans-bc feuille-sous-sans-bc feuille-directe \
           racine-inconnue feuille-inconnue; do
    openssl x509 -in "$TMP/$nom.pem" -outform der -out "$SORTIE/$nom.der"
    echo "tools/securite/$SORTIE/$nom.der"
done
