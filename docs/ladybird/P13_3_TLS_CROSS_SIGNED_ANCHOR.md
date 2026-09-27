# P13.3 — TLS : trust anchor cross-signée

## Preuve physique

`example.com` présente quatre certificats :

1. `example.com`
2. `Cloudflare TLS Issuing ECC CA 3`
3. `SSL.com TLS Transit ECC CA R2`
4. `SSL.com TLS ECC Root CA 2022`, variante cross-signée par `AAA Certificate Services`

Le certificat 4 et la racine locale `SSL.com TLS ECC Root CA 2022` ont le même
SPKI SHA-256 :

`634739f3820b0c9fdab238757d30e7802e5c1291ad9671b844c56de0171fd411`

Le validateur historique cherchait uniquement une racine dont le `subject`
correspond à l'`issuer` du dernier certificat. Il cherchait donc AAA et ignorait
que le dernier certificat transportait exactement la même identité de CA et la
même clé publique que la trust anchor SSL.com déjà présente dans le magasin.

## Correction

P13.3 accepte comme terminaison de chemin un certificat présenté dont :

- le `Subject` DER est identique à celui d'une trust anchor locale ;
- la clé publique est identique (RSA modulus/exponent ou point EC) ;
- tous les liens précédents de la chaîne ont un `issuer == subject` cohérent ;
- toutes les signatures précédentes sont cryptographiquement valides.

Le cross-signataire n'est pas ajouté au magasin et n'acquiert aucune confiance
implicite. Le chemin s'arrête sur la trust anchor locale équivalente.

## Critère physique final

Après rebuild/reflash :

- `tls_ok=true`
- `tls_trusted=true`
- `tls_hostname_ok=true`
- `tls_expired=false`
- `http_status=200`
- `http_complete=true`
- `chain_ok=true`
- `first_failure=aucun`
