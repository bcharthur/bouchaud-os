# P13.2 — TLS trust / SSL.com TLS 2022

## Preuve physique avant correction

Le 27/09/2026, le Trigkey a valide le chemin suivant vers `example.com` :

- bail DHCP obtenu ;
- ARP passerelle OK ;
- DNS OK ;
- TCP/80 et HTTP 200 OK ;
- TCP/443 OK ;
- handshake TLS 1.3 OK ;
- hostname X.509 OK ;
- certificat non expire ;
- HTTP chiffre 200 et document HTML complet ;
- mais `tls_trusted=false`, donc `chain_ok=false` et `first_failure=tls-certificat`.

La chaine publique servie par `example.com` en septembre 2026 remonte vers
`SSL.com TLS ECC Root CA 2022`. Le magasin embarque Bouchaud OS ne contenait
pas les racines TLS SSL.com 2022.

## Correction

Deux ancres TLS officielles SSL.com sont ajoutees :

- `SSL.com TLS ECC Root CA 2022`
  - SHA-256 `C32FFD9F46F936D16C3673990959434B9AD60AAFBB9E7CF33654F144CC1BA143`
- `SSL.com TLS RSA Root CA 2022`
  - SHA-256 `8FAF7D2E2CB4709BB8E0B33666BF75A5DD45B5DE480F8EA8D4BFE6BEBC17F2ED`

Les DER sont verifies par empreinte avant toute modification de `roots.rs`.

## Critere physique de fermeture

Apres reconstruction et reflash :

```text
tls_ok          = true
tls_trusted     = true
tls_hostname_ok = true
tls_expired     = false
http_status     = 200
http_complete   = true
chain_ok        = true
first_failure   = aucun
```

Aucune modification RTL8168, DHCP, ARP, DNS, TCP ou du handshake TLS n'est
necessaire pour ce lot.
