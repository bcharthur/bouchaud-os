# Bouchaud Lab -- le canal d'enquete a distance

## Pourquoi cet outil existe

Releve de la machine de reference, pendant la panne :

```
com1=bus-flottant  serial_bytes=0
rx_packets=64  rx_cur=0  desc_nic=64  desc_cpu=0   (pendant 151 s)
blackbox : session 395 s, persistance 193 s, checkpoints 0
```

Trois canaux d'enquete, et les trois absents au moment ou ils servaient. La
trace serie n'a pas produit un octet -- le port n'existe pas sur cette
machine. La reception meurt apres soixante-quatre trames, donc le reseau
ordinaire aussi. La boite noire s'arrete deux cents secondes avant la fin de
la session, donc le disque non plus.

Ce qui reste : **l'emission**. Le releve montre `chip_cmd = RX_ENB | TX_ENB`
et des trames qui partent pendant toute la panne. La machine ne peut plus
entendre, mais elle parle encore.

D'ou deux canaux, et pas un :

| canal | transport | sens | authentifie | survit a une RX morte |
|---|---|---|---|---|
| **BRDP** | TCP 2222 | bidirectionnel | oui, HMAC-SHA256 | **non** |
| **telemetrie** | UDP 2223, diffusion | emission seule | **non** | **oui** |

Un TCP sans reception ne s'etablit jamais : BRDP meurt avec la RX, et c'est
assume. La telemetrie n'a besoin que du sens qui reste vivant.

**Pour une campagne physique, lancer `telemetry --watch` AVANT d'allumer la
machine.** C'est le seul canal qui continuera de dire quelque chose apres la
panne, avec la boite noire.

## Installation

Aucune. Bibliotheque standard Python uniquement -- pas de `pip install`, pas
de dependance a installer le jour ou la machine tombe.

```powershell
python .\tools\remote\bouchaud-lab.py --help
```

```bash
python3 tools/remote/bouchaud-lab.py --help
```

## Le jeton

Le serveur BRDP ne demarre que si l'image a ete construite avec
`BOUCHAUD_DEBUG_TOKEN`. Le client a besoin du meme secret :

```powershell
$env:BOUCHAUD_DEBUG_TOKEN = "<le secret de l'image>"
python .\tools\remote\bouchaud-lab.py status --host 169.254.12.34
```

ou `--token <secret>`, qui l'emporte sur la variable.

**Le jeton ne traverse jamais le reseau.** Le serveur ouvre avec un nonce ; le
client prouve qu'il connait le secret en rendant `HMAC-SHA256(jeton, nonce)`.
Un nonce par connexion interdit le rejeu, et un HMAC faux ferme la connexion
-- laisser reessayer sur le meme nonce ferait de la fenetre d'authentification
un oracle.

Le client ne l'affiche nulle part : ni sur la sortie standard, ni sur l'erreur
standard, ni dans un fichier de dump, ni tronque dans un message d'erreur. Un
prefixe divise l'espace de recherche, ce qui est exactement ce qu'un secret ne
doit pas faire. Deux epreuves de `test_bouchaud_lab.py` le verifient.

### Ce que le jeton ne protege pas

Il est compile **en clair** dans l'image de laboratoire : `strings` sur cette
image le retrouve. C'est inherent a un secret partage lu a la construction.
La regle d'usage qui en decoule : **une image LAB ne se distribue pas.** Ce
que le jeton ferme, c'est l'acces depuis le segment local a quelqu'un qui n'a
pas l'image ; il ne ferme rien a quelqu'un qui l'a.

Une image construite **sans** jeton ne porte aucun code d'ecoute BRDP -- pas
le fil, pas la boucle d'acceptation, pas meme la chaine `bouchaud-brdp`.
`tools/ci/run_brdp_jeton.sh` le prouve sur le binaire.

## Trouver la machine

```bash
python3 tools/remote/bouchaud-lab.py discover --timeout 10
```

```
Bouchaud OS detected:
  host: 169.254.12.34
  telemetry: yes
  evenements recus: 37
```

**La decouverte est PASSIVE : aucun paquet n'est emis.** Sonder pour trouver
la machine reviendrait a dependre de sa reception -- la chose meme qui est en
panne le jour ou l'on cherche. L'outil se contente d'ecouter la diffusion que
la machine emet d'elle-meme. Une epreuve verifie qu'aucun datagramme ne sort.

L'adresse LAB est derivee de la MAC (`169.254.x.y/16`), donc stable d'un
demarrage a l'autre sans rien demander a DHCP.

## Les commandes

Toutes prennent `--host`, plus `--port` (2222), `--token`, `--timeout`.
`--json` est global et rend l'objet brut au lieu du texte lisible.

| commande | ligne BRDP envoyee | effet |
|---|---|---|
| `status` | `{"cmd":"status"}` | etat general, LAB, audit, RX, boite noire |
| `audit` | `{"cmd":"audit status"}` | l'auditeur LAB |
| `audit-run` | `{"cmd":"audit run"}` | force un tour d'auditeur |
| `audit-last` | `{"cmd":"audit last"}` | le dernier verdict |
| `net` | `{"cmd":"net status"}` | pile reseau, tri, etat des deux canaux |
| `rtl8168` | `{"cmd":"rtl8168 status"}` | tous les compteurs du pilote |
| `rtl8168-ring` | `{"cmd":"rtl8168 ring"}` | les 64 `opts1`, la carte `OWN` |
| `rtl8168-desc N` | `{"cmd":"rtl8168 desc","n":N}` | un descripteur, `0..63` |
| `dhcp` | `{"cmd":"dhcp status"}` | la negociation, etage par etage |
| `blackbox` | `{"cmd":"blackbox status"}` | persistance, checkpoints, retard |
| `checkpoint` | `{"cmd":"blackbox checkpoint"}` | **ECRIT** : force un checkpoint |
| `services` | `{"cmd":"services snapshot"}` | charge et temps |
| `processes` | `{"cmd":"processes snapshot"}` | taches et coeurs |
| `memory` | `{"cmd":"memory snapshot"}` | le tas |
| `serial-status` | `{"cmd":"serial status"}` | bornes de l'anneau serie RAM |
| `internet` | `{"cmd":"internet proof status"}` | etat de la preuve Internet |
| `internet-start` | `{"cmd":"internet proof start"}` | lance une generation de preuve Internet |
| `events --tail N` | `{"cmd":"events tail","n":N}` | les N derniers, `1..1024` |
| `events --watch` | `{"cmd":"events watch"}` | le flux continu |

<!-- BOUCHAUD_P0_REMOTE_CONTROL_V1_2_DOC -->
Les commandes de snapshot (`status`, `net`, `rtl8168`, `memory`, etc.) restent
en lecture seule. `checkpoint` a un effet de bord, et le plan de controle P0
ajoute des actions **bornees et nommees** : reboot/extinction, start/stop/restart
du navigateur et kill/kill-tree d'un PID explicite. Elles utilisent la meme
authentification HMAC et sont documentees dans [`REMOTE_CONTROL.md`](REMOTE_CONTROL.md).

Il n'y a **pas de shell**, et il n'y en aura pas : une commande arbitraire sur
un canal d'enquete est un acces root sur le segment local au premier jeton qui
fuit. Le controle distant est volontairement une liste fermee d'actions.

## Ce que `rtl8168` rend, et le critere de la campagne

```
python3 tools/remote/bouchaud-lab.py rtl8168 --host 169.254.12.34
```

Les compteurs sortent tels que le noyau les publie. **Aucun verdict n'est
calcule par le client** : les deux criteres viennent de la machine, sous le
nom que la campagne leur donne.

```
critere_rx_au_dela_de_64 : rx_packets > 64
critere_tour2            : rx_rendus_tour2 > 0
```

Les deux ensemble, et seulement les deux ensemble, diront que le second tour
de l'anneau RTL8168 demarre enfin. **La panne racine n'est pas corrigee a ce
jour.**

## Capture serie RAM via BRDP

```powershell
python .\tools\remote\bouchaud-lab.py serial-capture `
    --host 169.254.178.21 `
    --bytes 131072 `
    --out .\target\serial-live.log
```

La capture fige une fenetre de l'anneau serie RAM et ecrit aussi ses
metadonnees. Elle exige une RX fonctionnelle puisqu'elle passe par BRDP. Pour
une panne de reception, `telemetry --watch` lance **avant** l'essai reste le
canal de survie.

## La telemetrie

```bash
python3 tools/remote/bouchaud-lab.py telemetry --watch
```

```
[10:42:13.482] 169.254.12.34 [   12.482910] rtl8168 RING_WRAP_BEFORE rx_cur=63 ...
[10:42:13.487] 169.254.12.34 [   12.487002] audit AUDIT_RX_DMA_STALL ...
```

Format du fil, fige :

```
Ethernet  dst = ff:ff:ff:ff:ff:ff
IPv4      src = IP LAB (169.254.x.y)   dst = 169.254.255.255
UDP       src = 2223                   dst = 2223
charge    une ligne JSON par evenement, terminee par \n
```

La diffusion n'est pas un detail : emettre vers une adresse unicast demandrait
une resolution ARP, donc une REPONSE, c'est-a-dire de la reception. Faire
dependre le canal de survie de la chose en panne le rendrait inutile
exactement quand il sert.

### Ce canal n'est ni confidentiel ni authentifie

Il part en clair, en diffusion, sans signature. Quiconque est sur le segment
local le lit, et quiconque est sur le segment local peut forger un datagramme
qui lui ressemble. C'est assume, pour trois raisons qui tiennent **ensemble**
et pas separement :

- il n'existe qu'en LAB MODE, sur une image de laboratoire ;
- il ne sort pas du segment local : adresse link-local, pas de passerelle, pas
  de route par defaut ;
- **il n'accepte rien en entree.** C'est le point qui compte : un datagramme
  forge n'a personne a qui parler. Le seul socket qui ecoute est le BRDP en
  TCP, et celui-la est authentifie.

Ce qui fuit est donc l'etat interne d'une machine de laboratoire, a quiconque
a deja un acces physique a son commutateur. Ce qui ne peut pas arriver, c'est
qu'un tiers s'en serve pour agir sur la machine.

Un datagramme malforme ne tue pas l'ecoute : il est compte et jete. Un
listener qui meurt sur une ligne tronquee s'arrete exactement quand la machine
commence a mal aller.

## Le dump

```bash
python3 tools/remote/bouchaud-lab.py dump --host 169.254.12.34
```

Ecrit `target/remote-dump-AAAAMMJJ-HHMMSS/` :

```
metadata.json        host, date, version BRDP, les deux criteres, le compte
summary.json         status
audit.json           audit status
audit-last.json      audit last
net.json             net status
rtl8168.json         rtl8168 status
rtl8168-ring.json    rtl8168 ring
rtl8168-desc-0.json  le descripteur 0
rtl8168-desc-63.json le descripteur 63, celui du bouclage
dhcp.json            dhcp status
blackbox.json        blackbox status
services.json        processes.json        memory.json
events.jsonl         une ligne par evenement, les pertes comprises
```

**Le dump n'est pas tout-ou-rien.** Apres une panne RX, certaines reponses
manquent ; celles qui passent restent exploitables. Une commande refusee ecrit
un objet d'erreur dans SON fichier et les suivantes continuent. Meme une
connexion impossible laisse un `metadata.json` : « le client n'a pas pu se
connecter a telle heure » est deja un fait, et le jour de la panne c'est
souvent LE fait.

Le code de sortie distingue les trois cas : `0` complet, `2` partiel, `1`
aucune connexion.

Aucun fichier ne porte le jeton. Une epreuve relit tout le dossier pour s'en
assurer.

## Ce que le client garantit sur le fil

TCP est un flux : rien ne promet qu'une reponse arrive en un seul `recv`, ni
qu'un `recv` n'en contienne qu'une. Le lecteur du client est un recomposeur de
lignes **borne** -- un pair qui n'envoie jamais de terminateur ferait grandir
le tampon sans fin, la meme panne memoire que le serveur refuse de son cote.

Trois formes de ligne circulent, distinguees par leur premiere cle :

| cle | forme | sens |
|---|---|---|
| `ok` | `{"ok":true,"cmd":N,...}` | la reponse a une commande |
| `seq` | `{"seq":N,"t_ns":N,"cat":...}` | un evenement du catalogue |
| `lost` | `{"lost":N,"from":N,"to":N}` | un trou, annonce |

Les reponses ne sont **pas numerotees** : leur seul rattachement est l'ordre.
Le client verifie donc que le `cmd` rendu est celui qu'il attendait -- sinon il
lirait la reponse d'une autre commande, et tout ce qui suit serait decale.

Un `lost` n'est jamais masque. Un client qui presenterait une suite trouee
comme continue ferait tirer des conclusions fausses sur la chronologie.

## Campagne physique : l'ordre des gestes

```powershell
# 1. AVANT d'allumer. C'est le canal qui survivra.
python .\tools\remote\bouchaud-lab.py telemetry --watch
```

```powershell
# 2. Une fois la machine demarree, dans une autre fenetre.
python .\tools\remote\bouchaud-lab.py discover
python .\tools\remote\bouchaud-lab.py status  --host 169.254.x.y
python .\tools\remote\bouchaud-lab.py rtl8168 --host 169.254.x.y
python .\tools\remote\bouchaud-lab.py events --watch --host 169.254.x.y
```

```powershell
# 3. Des que quelque chose cloche, avant toute autre manipulation.
python .\tools\remote\bouchaud-lab.py dump --host 169.254.x.y
```

Si la reception meurt au soixante-quatrieme paquet, **BRDP peut mourir** :
plus de `status`, plus de `dump`, plus rien en TCP. C'est attendu, et ce n'est
pas une panne de l'outil.

**La telemetrie TX-only, elle, doit continuer**, et la boite noire aussi. Ce
sont ces deux traces-la qui devront enfin dire pourquoi le second tour de
l'anneau ne demarre pas -- `RING_WRAP_BEFORE`, `RING_WRAP_AFTER`,
`RING_SECOND_LAP_TIMEOUT`, `AUDIT_RX_DMA_STALL`, et la capture des
descripteurs 63 et 0 au moment du bouclage.

## Les epreuves

`tools/remote/test_bouchaud_lab.py` fait parler le client a un faux serveur
BRDP en boucle locale. Aucun reseau exterieur, aucun QEMU.

Le faux serveur sait faire ce que le vrai ne sait pas : fragmenter une annonce
octet par octet, coller trois reponses dans un seul envoi, fermer au milieu
d'une reponse, renvoyer du JSON invalide, ne jamais terminer une ligne. Ce
sont precisement les cas qu'un banc **avec** pile reseau reproduit le plus
mal, puisqu'on ne choisit pas comment le noyau distant segmente.

Lancees par `tools/ci/run_host_tests.sh`, decouvertes par repertoire : une
epreuve deposee dans `tools/remote` est bloquante le jour meme.
