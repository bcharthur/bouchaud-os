# M9 — HTTP distant via Ladybird RequestServer

## Base

M9 part du checkpoint :

- commit Bouchaud `b4d7b78` ;
- tag `resource-core-v1.1-m8-ok` ;
- Memory Fabric / VMA / Resource Core valides ;
- M8 reste une regression obligatoire.

Ladybird reste epingle au commit declare dans `third_party/UPSTREAM.md`.

## But

M9 ne remplace pas Ladybird par le client HTTP Rust de Bouchaud.

Le chemin valide est :

```text
WebContent
    |
    | IPC LibRequests
    v
RequestServer Ladybird
    |
    | socket/connect/send/recv
    v
ABI POSIX Bouchaud
    |
    v
TCP/IP Bouchaud
    |
    v
HTTP host fixture / Internet
```

La page recue revient ensuite dans le chemin normal de navigation LibWeb :

```text
HTTP response
  -> Fetch
  -> Navigable::navigate
  -> Document
  -> Layout/Paint
  -> screenshot CPU
  -> surface partagee
  -> Window Manager Bouchaud
```

M8, au contraire, conserve son injection HTML locale et son exit automatique.

## Processus M9

Le processus suivi par la fenetre reste `/bo-navigateur`, qui est le bootstrap.

Il engendre :

```text
bo-navigateur
  |- RequestServer   fd 101 (server)
  `- WebContent
       |- fd 100 : controle Browser minimal/factice
       `- fd 101 : RequestClient -> RequestServer
```

Le fd 100 est conserve pour ne pas reecrire tout le protocole Browser de
Ladybird en M9. Les rares decisions synchrones necessaires a une page HTTP
statique sont traitees localement dans `PageClient` :

- navigation process = `Local` ;
- cookie jar vide ;
- HSTS false pour le chemin Bouchaud M9.

Le vrai chargement reseau, lui, passe bien par RequestServer.

## Trois modes Windows

### Regression M8

```powershell
.\run.ps1 -LadybirdM8
```

Contrat fini : HTML local, capture, verdict, exit.

### M9 deterministe

```powershell
.\run.ps1 -LadybirdM9Test -RefreshLadybird
```

`run.ps1` lance une fixture HTTP Windows sur `0.0.0.0:18080`.
QEMU user networking l'expose au guest sous `10.0.2.2`.

URL :

```text
http://10.0.2.2:18080/m9.html
```

WebContent sort apres la frame verifiee et le bootstrap ferme RequestServer.

### M9 interactif

```powershell
.\run.ps1 -Ladybird -LadybirdUrl "http://example.com/"
```

Le document HTTP est charge et la fenetre reste vivante apres le premier rendu.
Fermer la fenetre Bouchaud termine le bootstrap et ses descendants.

Le mode interactif utilise directement le NAT sortant de QEMU et ne demarre pas
la fixture M9. Sans `-LadybirdUrl`, sa page de depart est
`http://example.com/`. La fixture locale est reservee a `-LadybirdM9Test`, dont
elle garantit le caractere reproductible.

On peut surcharger l'URL :

```powershell
.\run.ps1 -Ladybird -LadybirdUrl "http://10.0.2.2:18080/m9.html"
```

M9 valide HTTP. HTTPS/TLS distant est un jalon ulterieur si les certificats,
la compatibilite TLS RequestServer et l'horloge systeme doivent encore evoluer.

## Marqueurs

Le M9 test doit produire au minimum :

```text
M9_REQUESTSERVER_LAUNCHED
M9_REQUESTSERVER_READY
M9_REQUESTSERVER_CONNECTED
M9_NAVIGATION_BEGIN
M9_NAVIGATION_STARTED
M9_NAVIGATION_COMMITTED
M9_DOCUMENT_LOADED
M9_CAPTURE_MATCH
M9_CPU_SCREENSHOT_RENDERED
M9_GUI_HANDSHAKE_OK
M9_FRAME_READY_OK
M9_WEBCONTENT_EXIT_OK
RESULTAT : M9 HTTP distant dans fenetre Bouchaud OK
```

La fixture hote doit en plus produire :

```text
M9_FIXTURE_HTTP_OK path=/m9.html
```

Cette ligne est la preuve independante qu'une requete HTTP est reellement sortie
du guest. On n'utilise volontairement pas de faux marqueur
`M9_HTTP_RESPONSE_OK` sans instrumentation du RequestServer lui-meme.

En mode interactif, apres le rendu :

```text
M9_WEBCONTENT_STILL_ALIVE
```

## Build

Le patch Ladybird est applique uniquement au worktree jetable :

```text
prepare-browser-source.py
prepare-m9-source.py
prepare-browser-runtime-link.py
```

Le checkout epingle reste intact.

L'artefact M9 contient obligatoirement :

```text
WebContent
RequestServer
webcontent-bootstrap
M9_CAPABLE
resources/
```

`run.ps1` refuse donc silencieusement d'utiliser un ancien binaire M8 comme
RequestServer M9.

## CI

Le workflow `ladybird-native-browser` garde le job M8 existant et ajoute un job
M9 independant.

Le job M9 :

1. lance la fixture HTTP ;
2. construit une image avec WebContent + RequestServer ;
3. boot Bouchaud en QEMU ;
4. exige tous les marqueurs M9 ;
5. exige `M9_FIXTURE_HTTP_OK`.

M8 et M9 doivent etre verts avant merge.

## Perimetre

M9 couvre :

- RequestServer natif ;
- IPC RequestClient/RequestServer ;
- HTTP reel via le TCP/IP Bouchaud ;
- navigation LibWeb normale ;
- rendu du document distant dans la fenetre Bouchaud ;
- mode persistant ;
- fermeture par arbre de processus ;
- regression M8 conservee.

M9 ne pretend pas encore fournir :

- barre d'adresse complete ;
- multi-onglets ;
- ImageDecoder/WebWorker automatiques ;
- navigation dynamique via tous les messages Browser UI ;
- HTTPS universel/certificats ;
- isolation multi-processus de sites.

Ces elements viennent apres que le chemin HTTP fondamental est stable.
