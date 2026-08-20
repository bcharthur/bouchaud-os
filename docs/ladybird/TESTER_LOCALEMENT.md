# Tester le navigateur localement

Toutes les commandes ci-dessous se lancent depuis la racine du dépôt, sur la
machine de développement Windows. Elles ne compilent pas Ladybird : la
construction se fait sur GitHub Actions, et `run.ps1` récupère l'artefact.

---

## 1. Récupérer l'artefact d'une exécution CI

Ouvrir la branche dans Actions, prendre l'identifiant de l'exécution
`ladybird-native-browser` dont le job **WebContent-static-Bouchaud** est vert,
puis :

```powershell
.\run.ps1 -RefreshLadybird -LadybirdRunId <ID_DE_L_EXECUTION>
```

`-RefreshLadybird` retélécharge l'artefact ; sans lui, `run.ps1` réutilise celui
qui est déjà dans `third_party/native-browser-bouchaud`.

**Un artefact antérieur au 20 août 2026 ne convient plus** : il ne contient pas
`ImageDecoder`, et `run.ps1` s'arrête alors avec

```
artefact Ladybird incomplet : ImageDecoder absent
```

C'est voulu. Sans ce service, la première image d'un vrai site fait tomber
`VERIFY(s_the)` dans WebContent — mieux vaut le dire avant de démarrer QEMU.

---

## 2. Naviguer

Le chemin par défaut lance le bureau graphique. L'icône **Ladybird** est sur le
bureau ; la fenêtre porte une barre d'adresse, les boutons précédent / suivant /
recharger / arrêter, et un libellé d'état.

```powershell
.\run.ps1 -RefreshLadybird -LadybirdRunId <ID>
```

Pour ouvrir directement une adresse au démarrage :

```powershell
.\run.ps1 -RefreshLadybird -LadybirdRunId <ID> -LadybirdUrl "https://www.wikipedia.org/"
```

---

## 3. La suite fonctionnelle du moteur

C'est le test qui dit ce que le moteur sait faire — vingt-cinq vérifications :
DOM, style calculé, mise en page, événements, `localStorage`, `sessionStorage`,
décodage PNG / JPEG / WebP / GIF / SVG, canvas 2D avec relecture de pixel,
promesses, `setTimeout`, `fetch`, XHR, cookies.

Elle a besoin du serveur de fixture, servi depuis la machine hôte et joint
depuis Bouchaud à `10.0.2.2` :

```powershell
# Terminal 1 — la fixture
python tools\health\fixture_server.py --port 18081

# Terminal 2 — Bouchaud
.\run.ps1 -RefreshLadybird -LadybirdRunId <ID> -LadybirdUrl "http://10.0.2.2:18081/moteur.html"
```

Chaque vérification imprime une ligne sur la console série :

```
[ladybird-bouchaud] JS_CONSOLE log MOTEUR OK image_png_decodee
[ladybird-bouchaud] JS_CONSOLE log MOTEUR ECHEC canvas_webp_pixel 0,0,0
[ladybird-bouchaud] JS_CONSOLE log MOTEUR_BILAN reussis=23 total=25
```

La même page passe **25/25 dans Chromium** — vérifié avant d'écrire
l'assertion. Un échec ici est donc un défaut du portage, pas du test.

Pour la voir dans un navigateur de référence, sans Bouchaud :

```powershell
python tools\health\fixture_server.py --port 18081
# puis ouvrir http://127.0.0.1:18081/moteur.html
```

---

## 4. Lire ce qui se passe

Le journal série est le seul endroit où le portage parle. Les marqueurs utiles :

| Marqueur | Ce qu'il prouve |
|---|---|
| `IMAGE_DECODER_LANCE pid=…` | le service de décodage a démarré |
| `IMAGE_DECODER_CONNECTED pid=… fd=…` | le moteur a installé `ImageCodecPlugin` |
| `FONTCONFIG /usr/share/ladybird/fontconfig/fonts.conf` | le repli de polices de Skia est configuré |
| `M9_REQUESTSERVER_CONNECTED` | le réseau est branché au moteur |
| `M16_DNS_RX id=… rcode=0` | un nom a été résolu |
| `M9_NAVIGATION_COMMITTED page=1 url=…` | le document est arrivé et a été adopté |
| `M11_FIRST_FRAME pixels=…` | une trame est arrivée à l'écran |
| `JS_CONSOLE …` | la console JavaScript de la page |
| `JS_CONSOLE_ERREUR …` | une exception non rattrapée dans un script |
| `[syscall] non implemente : N (nom) appelant=… offset=0x…` | Ladybird demande quelque chose que Bouchaud n'a pas |
| `[cpu] faute de page en ring 3 : rip=… cr2=…` | un processus est mort, et où |

Le `[ps]` périodique donne la charge par processus. **Au repos, le navigateur ne
doit plus tenir le processeur** : depuis la correction du modèle de repeinture,
une page immobile ne déclenche aucune mise en page.

---

## 5. Trouver qui appelle un appel système manquant

Le message nomme désormais le processus et le déplacement dans son fichier :

```
[syscall] non implemente : 294 (inotify_init1) appelant=WebContent rip=0x… offset=0x2a1b40
```

Les binaires sont des PIE statiques chargés à une base connue, donc le
déplacement est directement une position dans le fichier :

```bash
addr2line -f -C -e third_party/native-browser-bouchaud/WebContent 0x2a1b40
```

---

## 6. Diagnostiquer une poignée de main TLS

Pour obtenir la trace complète de la négociation, poser la variable avant de
lancer le navigateur — depuis le shell Bouchaud, ou dans un scénario `autorun` :

```sh
export BOUCHAUD_CURL_TRACE=1
```

Chaque étape apparaît alors sur la console série :

```
[ladybird-bouchaud] CURL * TLSv1.3 (OUT), TLS handshake, Client hello (1):
[ladybird-bouchaud] CURL * TLSv1.3 (IN), TLS alert, ...
[ladybird-bouchaud] RS_ECHEC url=… code=35 (SSL connect error) pair=…:443 verif_cert=0 http=0
[ladybird-bouchaud] RS_ECHEC_DETAIL error:0A000410:SSL routines::sslv3 alert handshake failure
```

La ligne `RS_ECHEC_DETAIL` est celle qui manquait : sans
`CURLOPT_ERRORBUFFER`, curl ne pouvait rendre que le libellé générique de son
code d'erreur.

---

## 7. Vérifier une modification des scripts de préparation

Les divergences avec Ladybird vivent dans `tools/ladybird/prepare-*.py`, jamais
en modifications directes de l'arbre épinglé. Pour vérifier qu'une modification
s'applique encore, sans lancer la compilation complète :

```bash
./tools/ladybird/fetch.sh
git -C third_party/ladybird worktree add --force --detach /tmp/essai HEAD
for s in prepare-browser-source prepare-m9-source prepare-m9-diagnostics \
         prepare-m16-dns prepare-dns-une-question prepare-image-decoder \
         prepare-repaint prepare-tls-diagnostic prepare-browser-host \
         prepare-console prepare-m11-chrome prepare-browser-runtime-link; do
    python3 tools/ladybird/$s.py /tmp/essai || echo "ECHEC $s"
done
```

Un script qui ne trouve plus son ancre échoue bruyamment : c'est le signal d'une
divergence amont à examiner, pas à forcer.
