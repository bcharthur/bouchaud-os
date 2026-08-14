# Portage de RequestServer

## Etat actuel

Bouchaud possede une pile reseau complete cote noyau : Ethernet e1000, ARP, IPv4,
ICMP, UDP, TCP (maison + smoltcp en second chemin), DHCP, DNS, HTTP/1.1, HTTP/2
(HPACK), et un TLS ecrit sur place (X25519, P-256/384, RSA, AES-GCM, ChaCha20,
SHA-2, X.509).

Cote userland, le navigateur a un courtier reseau asynchrone (`moteur/transport.py`).

## Ce que RequestServer demande

`Services/RequestServer` s'appuie sur **curl** (`vcpkg.json` : curl 8.21 avec
brotli, http2, http3, openssl, websockets, zstd). C'est un fait structurant :
Ladybird ne parle pas HTTP lui-meme, il delegue.

Deux voies s'offrent, et le choix n'est pas encore tranche :

**Voie 1 — porter curl + OpenSSL.** Fidele a upstream, aucune divergence, mais
c'est porter deux gros paquets qui font doublon avec notre pile TLS.

**Voie 2 — implementation Bouchaud de l'interface RequestServer.** On garde
l'interface IPC de RequestServer et on la sert avec notre pile. Divergence
maitrisee, mais divergence quand meme : chaque evolution d'upstream doit etre
reportee.

Critere de decision, a etablir avant M11 : si OpenSSL doit de toute facon etre
porte pour `LibCrypto` (il le doit — liaison **publique**), alors le surcout de
curl est faible et la voie 1 l'emporte. La question se reduit donc a : **peut-on
porter OpenSSL ?** Elle est deja sur le chemin critique de LibJS.

## Correspondance

| Ladybird | Bouchaud | Etat |
|---|---|---|
| `socket`/`connect` | pile TCP noyau | supporte |
| DNS | resolveur noyau + `/etc/resolv.conf` | supporte |
| TLS | TLS maison, ou OpenSSL porte | partiel |
| HTTP/1.1 | present | supporte |
| HTTP/2 | HPACK present | partiel |
| HTTP/3 (QUIC) | absent | absent |
| WebSocket | present cote moteur actuel | partiel |
| Cache disque | RAMFS + persistance | partiel |

## Etapes

1. RequestServer demarre et repond en IPC (M11), sans reseau.
2. HTTP sur **fixture locale** — jamais Internet dans un test bloquant. Le depot
   a deja `navigateur/serveur_test.py` pour cela.
3. HTTPS sur fixture locale avec autorite fabriquee (le mecanisme existe deja).
4. `https://example.com` en sonde **informative**, jamais bloquante.

## Regle heritee de la CI actuelle

Aucun travail bloquant ne depend d'Internet. Elle s'applique integralement a
RequestServer.
