# Couche reseau Bouchaud OS — recapitulatif (cloture)

Cette section documente la pile reseau, considere **complete pour le perimetre
realiste from-scratch**. Seul TLS (HTTPS) reste un chantier crypto dedie.

## Pile implementee (reelle, testee en QEMU)

| Couche | Module | Etat |
|--------|--------|------|
| Pilote NIC | `drivers/e1000.rs` | reel (MMIO, DMA RX/TX, MAC) |
| L2 Ethernet | `net/ethernet.rs` | reel |
| ARP | `net/arp.rs` | reel (resolution + cache passerelle) |
| L3 IPv4 | `net/ipv4.rs` | reel (+ checksum) |
| ICMP | `net/icmp.rs` | reel (`ping` loopback **et** externe) |
| L4 UDP | `net/udp.rs` | reel |
| DNS | `net/dns.rs` | reel (`dns <nom>`) |
| DHCP | `net/dhcp.rs` | reel (`dhcp` : IP/gw/dns auto) |
| L4 TCP | `net/tcp.rs` | client minimal (poignee de main, ACK, FIN) |
| L7 HTTP | `net/http.rs` | GET HTTP/1.0 (`wget`/`http`, navigateur) |
| TLS / HTTPS | `net/tls.rs` | **socle uniquement** (handshake non implemente) |

Config dynamique : `net::our_ip()`, `gateway()`, `dns_server()`, modifiables par
DHCP via `net::set_config()`.

## Commandes

```
ifup                 active eth0 (driver e1000)
ethinfo              etat de la carte (MAC, lien)
dhcp                 configuration automatique (DORA)
ifconfig / ip        etat des interfaces
arping <ip>          requete ARP
ping <ip>            ICMP (loopback ou reseau reel)
dns <nom>            resolution DNS
wget / http <url>    requete HTTP (http://)
route / arp          tables
```

## Test type (QEMU, via run.ps1 qui cable deja une e1000)

```
ifup
dhcp                 -> bail obtenu inet 10.0.2.15 gw 10.0.2.2 dns 10.0.2.3
ping 8.8.8.8         -> 4 reponses
dns example.com      -> 93.184.216.34
wget http://example.com
```

## HTTPS / TLS : pourquoi c'est laisse en chantier

Un client TLS fonctionnel exige une **pile cryptographique complete et exacte** :
echange de cles (X25519 / ECDHE P-256), chiffrement authentifie (AES-GCM /
ChaCha20-Poly1305), hachage (SHA-256/384, HMAC, HKDF), signatures (RSA/ECDSA),
parsing **et validation** X.509/ASN.1 contre un magasin de CA, plus une source
d'entropie. C'est un projet de plusieurs milliers de lignes auditees ; le simuler
echouerait au premier handshake. Il est donc **isole** dans `net/tls.rs` (cadre
record + types) et constituera un chantier dedie ulterieur.

`https://` dans le navigateur et `wget` affichent un message honnete.
