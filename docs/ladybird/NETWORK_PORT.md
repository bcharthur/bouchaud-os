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

Critere de decision, a etablir avant M9 : si OpenSSL doit de toute facon etre
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

1. RequestServer demarre et repond en IPC (M9), sans reseau.
2. HTTP sur **fixture locale** — jamais Internet dans un test bloquant. Le depot
   a deja `navigateur/serveur_test.py` pour cela.
3. HTTPS sur fixture locale avec autorite fabriquee (le mecanisme existe deja).
4. `https://example.com` en sonde **informative**, jamais bloquante.

## Un obstacle decouvert en construisant LibGC (PR 8)

L'edition de liens de `LibCore/System.cpp` en statique produit :

    warning: Using 'getaddrinfo' in statically linked applications requires at
    runtime the shared libraries from the glibc version used for linking

Ce n'est pas cosmetique. `getaddrinfo` de la glibc delegue a **NSS**, qui charge
des greffons (`libnss_dns.so`, `libnss_files.so`) par `dlopen` a l'execution.
Dans un binaire statique-PIE deploye sur Bouchaud, ces greffons n'existent pas :
la resolution de noms echouera, silencieusement ou par une erreur sans rapport.

Trois issues, a trancher avant M9 :

1. **Ne pas utiliser `getaddrinfo`.** RequestServer passe par curl, qui peut
   etre construit avec son propre resolveur (`--enable-ares`) ou pointe sur le
   notre.
2. **Fournir la resolution nous-memes**, en interceptant l'appel : Bouchaud a
   deja un resolveur DNS noyau.
3. **Passer a musl**, dont le `getaddrinfo` est autonome et fonctionne en
   statique. C'est la raison technique la plus solide de reconsiderer musl pour
   le userland — et elle n'etait pas visible avant de lier reellement.

Le point est sans effet sur LibGC et LibJS, qui n'appellent jamais `getaddrinfo`.
Il est note ici parce que c'est la que la decision se prendra.

## Regle heritee de la CI actuelle

Aucun travail bloquant ne depend d'Internet. Elle s'applique integralement a
RequestServer.
