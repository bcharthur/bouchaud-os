# TLS 1.3 from-scratch (HTTPS) — Bouchaud OS

Implémentation **complète et fonctionnelle** d'un client TLS 1.3 (RFC 8446),
écrite intégralement à la main en `no_std` + `alloc`, sans aucune dépendance
cryptographique externe. Permet de récupérer des pages **`https://`** réelles
depuis le shell ou le navigateur intégré.

## Périmètre cryptographique

| Brique | Fichier | Détail | Vecteurs de test |
|--------|---------|--------|------------------|
| SHA-256 / HMAC / HKDF | `sha256.rs` | + HKDF-Expand-Label TLS 1.3 | FIPS-180, RFC 4231, RFC 5869 |
| SHA-384 / SHA-512 | `sha512.rs` | SHA-384 pour ECDSA P-384 | FIPS-180 |
| AES-128 / AES-256 | `aes.rs` | chiffrement de bloc (FIPS-197) | FIPS-197 App. C |
| AES-GCM (AEAD) | `gcm.rs` | GHASH GF(2^128) + CTR | NIST SP 800-38D |
| X25519 | `x25519.rs` | ECDH Curve25519 (radix 2^51) | RFC 7748 §5.2 |
| P-256 / ECDSA | `p256.rs` | corps premier, jacobien, vérif ECDSA | RFC 6979 A.2.5 |
| P-384 / ECDSA | `p384.rs` | corps premier, jacobien, vérif ECDSA SHA-384 | SEC 2 / FIPS-186 |
| RSA | `bignum.rs`, `rsa.rs` | grands entiers + PKCS#1 v1.5 + PSS | OpenSSL |
| ASN.1 / DER | `asn1.rs` | parseur TLV | — |
| X.509 | `x509.rs` | clé publique, validité, SAN, basicConstraints | vrais certs |
| Magasin de CA | `roots.rs`, `ca/` | 6 racines Mozilla (DER embarqués) | — |
| Validation chaîne | `validate.rs` | signatures + ancre + hostname + dates | vrais certs |
| CSPRNG | `rng.rs` | RDRAND + TSC → HASH-DRBG | — |
| AES-GCM + ChaCha20-Poly1305 | `gcm.rs`, `chacha.rs` | 3 suites TLS 1.3 | NIST SP 800-38D, RFC 8439 |
| Alertes TLS | `alert.rs` | codes RFC 8446 §6 → noms lisibles | RFC 8446 §6 |
| Couche record | `record.rs` | protection AEAD + key schedule | RFC 8446 §7 |
| Handshake | `handshake.rs` | machine d'état client | — |

## Déroulé du handshake (client)

1. Génération d'une paire éphémère **X25519** ; envoi du **ClientHello**
   (SNI, `supported_versions=TLS 1.3`, `supported_groups=x25519`,
   `signature_algorithms`, `application_layer_protocol_negotiation=http/1.1`,
   `key_share`). Trois suites offertes : `TLS_AES_128_GCM_SHA256`,
   `TLS_AES_256_GCM_SHA384` et `TLS_CHACHA20_POLY1305_SHA256` ; la fonction de
   hachage (SHA-256/SHA-384) suit la suite négociée par le serveur.
2. Réception du **ServerHello**, extraction du `key_share` serveur.
3. Calcul du secret partagé **ECDHE** puis du *key schedule* HKDF
   (early → handshake → master secrets, trafic client/serveur).
4. Déchiffrement (AEAD de la suite négociée) du *flight* serveur :
   EncryptedExtensions, **Certificate**, **CertificateVerify**, **Finished**.
   Une **alerte TLS** reçue à cette étape est décodée en nom lisible
   (`handshake_failure`, `unknown_ca`, `protocol_version`...) via `alert.rs`.
5. **Vérification de la signature CertificateVerify** avec la clé publique du
   certificat feuille (RSA-PSS / RSA-PKCS#1 / ECDSA P-256).
6. **Vérification du `Finished` serveur** (HMAC sur le hash de transcript).
7. **Validation de la chaîne X.509** : chaque certificat est vérifié contre le
   suivant, l'ancre est cherchée dans le magasin de racines embarqué, le nom
   d'hôte est comparé aux SAN et les dates de validité contrôlées (RTC).
8. Envoi du **Finished** client, passage aux clés applicatives, requête HTTP
   chiffrée et déchiffrement de la réponse.

## Commandes shell

```
tls-selftest          # valide toute la crypto par vecteurs de référence
tls                   # diagnostic : suite, groupe, magasin de CA
tls example.com       # exécute un vrai handshake TLS 1.3 et affiche l'état
https example.com     # GET https:// réel (équiv. wget https://...)
wget https://hote/chemin
```

Le bandeau `[TLS OK]` / `[TLS !]` indique le résultat de la validation de
chaîne (confiance, correspondance du nom d'hôte, expiration).

## Validation effectuée

- **Tous les primitifs** passent leurs vecteurs de référence (RFC/NIST/FIPS).
- **RSA** (PKCS#1 v1.5 + PSS) vérifié contre des signatures OpenSSL.
- **X.509 + chaîne RSA** validés contre de vrais certificats.
- **Handshake complet** testé de bout en bout contre un serveur TLS 1.3 réel :
  ClientHello → ServerHello → ECDHE → déchiffrement du flight →
  CertificateVerify + Finished vérifiés → échange HTTP chiffré déchiffré.

## Limites connues

- Trois suites TLS 1.3 (`AES-128-GCM`, `AES-256-GCM`, `ChaCha20-Poly1305`) mais
  un seul groupe (`x25519`) offert — suffisant pour la quasi-totalité des
  serveurs publics.
- Validation de chaîne : **RSA (SHA-256)**, **ECDSA P-256 (SHA-256)** et
  **ECDSA P-384 (SHA-384)** sont supportés. Cela couvre les chaînes modernes
  courantes (ex. Google rootées sur *GTS Root R4*, ou Let's Encrypt ECDSA
  rooté sur *ISRG Root X2*) afin que le bandeau puisse afficher `[TLS OK]`
  quand l'ancre, le nom d'hôte et la validité temporelle passent.
- Le contenu est affiché même quand la chaîne n'est pas approuvée (comme un
  navigateur affichant un avertissement) ; le bandeau distingue les deux cas.
- Les requêtes HTTPS annoncent `Accept-Encoding: gzip, deflate` (décodés par
  `net::inflate`) et, si un domaine apex minimal comme `google.com` termine TLS
  sans envoyer de réponse HTTP, le client retente automatiquement le frontend
  canonique `www.<domaine>`.
- Pas de reprise de session (PSK / 0-RTT), pas de HelloRetryRequest
  (inutile puisque x25519 est universellement supporté).
- Implémentation **non *constant-time*** : objectif pédagogique, pas de
  résistance aux attaques par canaux auxiliaires.
- Magasin de racines : ~19 ancres courantes (RSA + quelques ECDSA) ; en ajouter
  via `ca/*.der` + `roots.rs`.

## Pistes d'évolution

- ECDHE P-256 en plus de x25519 pour le `key_share` (HelloRetryRequest).
- Reprise de session (PSK / NewSessionTicket / 0-RTT).
- OCSP stapling / Certificate Transparency (SCT).
