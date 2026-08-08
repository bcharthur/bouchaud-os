# Audit indépendant de Bouchaud Browser

**État audité :** branche `work`, commit `4cc03d0`, 8 août 2026.
**Méthode :** lecture du code exécutable (et non des README), exécution de la
suite locale avec le vrai QuickJS, inspection des chemins de construction et
sondes ciblées. Les affirmations ci-dessous distinguent **observé**, **testé**
et **non testé**. Cet audit n'est pas une promesse de conformité.

## Résumé exécutif

Le navigateur réellement lancé est `/bo-navigateur`: un unique processus ring
3 statique qui réunit Qt 5/QPainter, CPython, le moteur Python et QuickJS. Le
moteur est réel et substantiel (HTML, cascade, layout, display list, DOM/JS),
mais sa sécurité Web n'a pas encore le socle nécessaire à une navigation sur du
contenu non fiable. Le défaut bloquant est l'absence de modèle d'origin appliqué
aux chargements : `fetch`, XHR, scripts/modules, images et feuilles passent
directement à `reseau.charge`. Il n'y a ni SOP, ni CORS, ni preflight, ni mixed
content. Les cookies sont joints globalement par destination, y compris aux
requêtes cross-origin initiées par une page.

**Conclusion de release : P0 / ne pas présenter comme navigateur sûr.** Le bac
à sable QuickJS empêche l'accès direct aux API hôte, mais ne remplace pas la
sécurité Web. Une page hostile peut lire des réponses authentifiées d'une autre
origine et monopoliser le processus qui porte aussi la GUI.

## 1. Architecture réelle actuelle

Chemin actif vérifié :

```text
window manager -> exec /bo-navigateur (ring 3, un processus)
  hote.cpp: QApplication/QPainter + CPython embarqué + modules bo/bojs/bomedia
    navigateur.py: chrome, onglets, événements, navigation, boucle de trames
      moteur/__init__.py: Document et orchestration
        reseau.py -> socket/ssl/http.client -> syscalls réseau du noyau
        html.py -> arbre Element/Texte
        css.py -> analyse, sélecteurs, cascade
        js.py + prelude.js -> pont DOM/fetch/stockage -> QuickJS (bojs.cpp)
        mise_en_page.py (+ flex.py, grille.py) -> arbre de boîtes
        peinture.py -> display list
      hote.cpp -> QPainter -> framebuffer
```

`Document.__init__` précharge, analyse les feuilles, crée QuickJS seulement si
un `<script>` existe, exécute les scripts, fait un layout complet, signale
`DOMContentLoaded/load`, puis peut refaire le layout. Toutes les mutations DOM
et de style posent le seul booléen `Contexte.sale`. Au battement suivant,
`Document.rafraichis()` reparcourt les feuilles, puis appelle
`remet_en_page()` pour le document entier. `liste_affichage()` reconstruit la
liste de peinture ; la peinture élague les boîtes hors viewport.

### Composants actifs, auxiliaires et obsolètes

| Composant | Statut constaté | Preuve dans le code |
|---|---|---|
| `tools/userland/navigateur/{hote.cpp,navigateur.py,moteur/}` | **actif, navigateur principal** | construit par `build-navigateur.sh`; chemin par défaut injecté par `hote.cpp`; lancé par le window manager |
| `bojs.cpp` + QuickJS 2021-03-27 | **actif** | module embarqué utilisé par `moteur/js.py` |
| `bomedia.cpp` + FFmpeg | **actif optionnel à la construction** | module embarqué et utilisé par `moteur/media.py` |
| `apercu.py` | **outil actif développeur** | rejoue la display list avec Pillow; substitue `curl` au transport OS |
| `webview_bouchaud.py`, greffe pywebview, exemple | **adaptateur/tutoriel**, pas le chrome principal | deuxième programme Python accepté par le même hôte |
| `moteur/distant.py`, `tools/render-proxy` | **mode distant auxiliaire** | image rendue par Chromium hôte; ce n'est pas le moteur natif |
| `tools/qt-browser/browser.py` | **prototype hôte QtWebEngine** | hors artefact `/bo-navigateur`; ne valide pas le moteur natif |
| commande kernel `pybrowser`, `src/assets/python/browser.py` | **legacy** | navigateur simplifié encore exposé par le shell, distinct du ring 3 |
| `src/net/application/html.rs` et Nautile ring 0 | **ancien moteur/prototype** | non appelé par `/bo-navigateur`; le nom Nautile subsiste dans diagnostics/User-Agent |

Organisation cible sans suppression immédiate : garder le moteur actif sous
`tools/userland/navigateur`, déplacer ultérieurement les démonstrations dans
`examples/`, les sondes dans `tests/probes/`, le proxy Chromium dans
`tools/remote-renderer/`, et marquer `pybrowser`, `qt-browser` et l'ancien HTML
kernel dans un manifeste `legacy/README.md` avant toute suppression.

## 2. Frontières processus, rings, filesystem et réseau

* **Ring :** `/bo-navigateur` est un ELF ring 3. Les faults CPU ring 3 sont
  converties en terminaison de processus par l'IDT. Le noyau possède une PML4
  par processus, les syscalls, `kill`, signaux, `fork/exec`, `socketpair` et
  mémoire partagée.
* **Processus :** GUI Qt, Python, QuickJS de tous les onglets, media et moteur
  vivent dans **le même processus**. Un contexte QuickJS existe par Document,
  mais ce n'est pas une frontière mémoire ou d'ordonnancement.
* **Filesystem :** JavaScript n'obtient pas `std`, `os`, `require` ou `process`.
  En revanche le pont de page permet `file://` via le chargeur, et
  `_charge_fichier` ouvre tout chemin transmis. Une page HTTP peut donc demander
  `fetch("file:///...")`: aucune règle de scheme/origin ne l'arrête. **P0.**
  Cookies, cache et localStorage sont écrits sous
  `$BO_PERSIST` (défaut `/persist/bo-navigateur`) sans quota par origin.
* **Réseau :** le code Python ouvre UDP DNS et TCP directement via libc/syscalls.
  Le noyau porte Ethernet/ARP/IPv4/UDP/TCP et les descripteurs. TLS est celui de
  Python/OpenSSL dans le navigateur ring 3, pas `src/net/security/tls`.

## 3. Sécurité Web (priorité maximale)

### Matrice de conformité observée

| Contrôle | État | Conséquence |
|---|---|---|
| tuple Origin (scheme, host, port) | calculé seulement pour `location.origin` et localStorage | aucune décision d'accès centralisée |
| Same-Origin Policy | **absente** | lecture cross-origin par fetch/XHR |
| CORS / ACAO / ACAC | **absents** | toute réponse est exposée au script |
| preflight OPTIONS | **absent** | méthodes/headers non simples partent directement |
| Fetch `credentials` | **ignoré** | cookies joints selon la destination, même si `omit`; cross-origin par défaut |
| cookies Secure | partiel | non envoyés en HTTP |
| cookies HttpOnly | **attribut jeté** | `document.cookie` expose les cookies serveur HttpOnly |
| cookies SameSite | **ignoré** | CSRF facilité; commentaire du code fondé sur l'hypothèse fausse qu'il n'y a pas de requête croisée |
| cookies Domain | partiel | rejet d'un Domain sans suffixe correspondant; pas de Public Suffix List; host-only non mémorisé |
| cookies Path | partiel | simple `startswith`, donc `/admin` correspond à `/administrator` à tort |
| localStorage | origin scheme/`netloc` | isolation bank/evil correcte pour HTTP(S); toutes les URL sans hostname deviennent `about`; aucun quota |
| iframe | **non implémenté comme browsing context isolé** | pas de `contentWindow`/document enfant; aucune politique sandbox/SOP à tester |
| modules JS cross-origin | **sans CORS** | import HTTP(S) arbitraire et source exposée à QuickJS |
| canvas cross-origin | **pas de taint tracking** | une image cross-origin dessinée peut être relue par `getImageData` |
| mixed content | **absent** | une page HTTPS charge scripts/images/fetch HTTP |
| CSP, SRI, Referrer Policy | absents | aucune défense en profondeur |
| validation TLS | dangereusement conditionnelle | si le magasin CA est vide, hostname et certificat sont volontairement désactivés |

### Défauts confirmés et tests adversariaux

Ces quatre tests sont **des tests de sécurité attendus qui échouent** sur le
code audité. Ils sont reproductibles sans Internet en remplaçant
`reseau.charge` par une réponse fictive, comme le fait la suite existante.

#### SEC-01 — lecture cross-origin authentifiée (critique)

**Reproduction :** construire un `Document` à
`https://evil.example/index`, remplacer `reseau.charge` par une fonction qui
rend `Reponse("https://bank.example/balance", '{"balance":100000}', ...)`, puis
exécuter `fetch('https://bank.example/balance').then(r=>r.text())`. Deux appels
à `tic()` livrent le texte au script. Variante XHR identique.

**Résultat attendu / test échouant :** la Promise doit rejeter avec `TypeError`
en l'absence d'un `Access-Control-Allow-Origin` valide; actuellement elle est
résolue et le corps est lisible. Par inspection, `_op_requete` fait seulement
`urljoin` puis `_recupere`; `_recupere` retourne corps et tous les headers sans
contrôle. `_charge_http` ajoute en outre les cookies de `bank.example`.

**Cause racine :** aucune classe `Origin`, aucun mode de requête, aucune
validation CORS, aucun filtrage des response headers, et cookies ajoutés trop
bas dans le transport. **Sévérité : critique.**

**Correctif recommandé :** introduire une couche `web_security.py` entre JS et
transport : Origin canonique, request mode/destination/credentials, liste des
méthodes/headers simples, cache de preflight, validation ACAO/ACAC, réponse
opaque et liste CORS-safelisted. Le transport ne doit plus décider seul des
credentials. Seconde review obligatoire.

#### SEC-02 — HttpOnly exposé à JavaScript (élevée)

**Reproduction :** `Temoins.absorbe('https://bank.example/',
{'set-cookie':'session=secret; HttpOnly; Secure; Path=/'})`, puis lire
`document.cookie` depuis un document bank. Le parseur `_analyse_temoin` ne crée
aucun champ `httponly`; `_op_temoins` appelle le même `pour()` que l'en-tête
réseau et rend `session=secret`.

**Test échouant :** `assert 'session=' not in document.cookie`; obtenu : secret
visible. **Cause :** absence de métadonnée HttpOnly et API unique pour réseau et
script. **Sévérité : élevée.**

**Correctif recommandé (petit mais à coordonner) :** mémoriser `httponly`, faire
`pour(url, javascript=False)` et exclure les HttpOnly en mode JavaScript;
interdire à `document.cookie` de créer HttpOnly. Ajouter migration tolérante du
JSON existant.

#### SEC-03 — canvas cross-origin lisible (élevée)

**Reproduction :** page evil, image issue de bank, `drawImage(image,...)`, puis
`getImageData`. `_toiles` ne conserve que les opérations; `_op_rasterise` les
envoie à Qt et retourne toujours les pixels RGBA. L'objet image et les
opérations ne portent aucune origin/CORS status.

**Test échouant :** `getImageData` devrait lever `SecurityError`; actuellement
il rend les pixels. **Cause :** absence de provenance des ressources et de bit
`origin-clean` sur canvas. **Sévérité : élevée.**

**Correctif recommandé :** attacher origin + résultat CORS à chaque image,
faire passer ce statut par `drawImage`, rendre le canvas définitivement
non-origin-clean au premier dessin contaminant, et bloquer
`getImageData`/`toDataURL`. Ce travail dépend de SEC-01.

#### SEC-04 — HTTPS vers HTTP / file (critique)

**Reproduction :** depuis `https://evil.example`, `fetch('file:///persist/...')`
ou depuis une page HTTPS charger `<script src="http://...">`. `urljoin` accepte
la cible et `reseau.charge` route respectivement vers `_charge_fichier` ou TCP
clair.

**Test échouant :** les deux chargements actifs doivent être bloqués avant I/O;
ils ne le sont pas. **Cause :** aucune policy de scheme ni destination.
**Sévérité : critique** pour `file://`, élevée pour mixed active content.

**Correctif recommandé immédiat minimal :** refuser `file://` dans toutes les
requêtes initiées par un Document réseau et refuser HTTP actif depuis HTTPS.
La policy complète doit ensuite distinguer navigation, image, media, style,
script, module, fetch et téléchargement.

### Autres constats cookies/storage

L'isolation `localStorage` demandée (`evil.example` ne lit pas bank) passe dans
la suite actuelle, y compris la séparation par port. Ce résultat ne couvre pas
les domaines IDN, IPv6, origins opaques, `file:`, quotas, événements `storage`
entre contexts ni concurrence d'écriture. Les cookies host-only deviennent en
pratique des domain cookies parce que ce statut n'est pas stocké; sans Public
Suffix List, un serveur sur un suffixe public contrôlable peut élargir trop un
cookie. `SameSite` ne peut être corrigé proprement sans transmettre le
top-level site et le type de navigation à la cookie jar.

## 4. Page hostile et isolation

| Charge hostile | Impact exact actuel |
|---|---|
| `while(true){}` | QuickJS est créé avec `budget_ms=5000`; le handler d'interruption arrête l'évaluation après le budget. Pendant ce temps, le thread GUI/processus est bloqué jusqu'à ~5 s. L'exception est journalisée, le Document peut survivre. |
| mémoire JS massive | `bojs.cpp` pose une limite mémoire au runtime (128 MiB observés dans le code); OOM QuickJS reste dans le processus mais la gestion d'erreur n'est pas testée end-to-end sous OS. |
| DOM massif | nœuds Python sans quota; parse/cascade/layout et dictionnaires peuvent épuiser le tas du processus. Pas de limite par page/onglet. |
| timers en boucle | map non bornée; chaque `tic()` parcourt et exécute les timers échus sur le thread GUI. Le budget est par entrée QuickJS, pas un budget global de frame. |
| exception QuickJS | `Contexte.execute/_appelle` journalise et continue; faute native de QuickJS/extension C++ termine le processus entier. |
| relayout permanent | toute mutation pose `sale`; un layout complet est fait au prochain battement; animation active force aussi un layout complet par frame. GUI saccadée, navigateur entier affecté. |

Le noyau ne devrait pas tomber : les faults ring 3 terminent le processus. En
revanche la GUI du navigateur, tous ses onglets et décodages tombent ensemble;
le desktop devrait rester vivant. Ceci est une conclusion du modèle de
processus, pas un crash-test QEMU réalisé pendant cet audit.

### Minimum pour isoler un renderer (ne pas l'implémenter sans review)

1. un browser process possédant chrome, navigation, cookies et réseau;
2. un renderer ring 3 par site instance ou onglet, sans accès direct aux
   sockets ni au filesystem persistant;
3. IPC cadré (messages bornés, IDs de navigation, validation systématique) sur
   `socketpair` ou pipes déjà disponibles;
4. surface partagée double-buffer ou display list sérialisée bornée;
5. quotas/compteurs mémoire par processus, kill fiable, détection de sortie et
   page « renderer crashed »;
6. scheduler préemptif et watchdog hors renderer; limites CPU/timers;
7. sandbox de syscalls/capabilities : l'existence du ring 3 seule ne retire pas
   `open` et `socket`.

Le noyau semble déjà fournir processus, espaces mémoire, fork/exec, signaux,
kill, socketpair et shared memory via ses sondes POSIX. Il manque à **prouver** :
accounting/limite mémoire par processus, récupération garantie de toutes les
pages à la mort, handles partagés sûrs, attente non bloquante robuste et
restriction de capabilities/syscalls.

## 5. Compatibilité Web

### Ce qui est réellement exercé

HTML tolérant basique, entités, raw text script; sélecteurs avancés; cascade,
variables et media queries; bloc/inline, flex, grid, tables et positionnement;
display list; DOM courant, events, timers, Promises, Mutation/Intersection
Observer; Web Components/shadow DOM partiel; modules QuickJS; fetch/XHR de
forme minimale; canvas 2D; images; animations/transitions; media/MSE partiel;
cookies, cache et localStorage.

### Limites structurantes

Ce n'est pas un moteur conforme HTML Living Standard : parseur, navigation,
browsing contexts, event loop, encodages et chargement des ressources sont des
sous-ensembles maison. Pas de véritable iframe, workers/service workers,
WebSocket, History complet, IndexedDB, CSP, permissions, accessibility tree,
print ni HTTP/2 utilisé par le client. QuickJS date de 2021 et ne représente
pas les versions ECMAScript modernes. L'API `fetch` ignore notamment mode,
credentials, redirect, cache, signal/abort et streams; XHR `abort` est vide et
`timeout` n'est pas appliqué.

Les pages « réelles » peuvent fonctionner visuellement tout en violant des
invariants Web. Le nombre de propriétés CSS n'est donc pas la métrique à
optimiser avant sécurité, navigation, event loop et tests différentiels.

## 6. Qualité des tests et mesure

Commande exécutée : `./tools/userland/test-moteur.sh`.

* **575 assertions passent**, regroupées dans **69 scénarios** nommés après
  l'ajout du diagnostic runtime de compatibilité.
* Le vrai QuickJS 2021-03-27 est compilé et utilisé; CPython 3.14.4; l'hôte Qt
  est remplacé par un stub. Tous les groupes qui construisent un `Document`
  scripté ou un `Contexte` utilisent donc QuickJS réel. La suite ne publie pas
  actuellement un décompte automatique « avec QuickJS » par assertion.
* Ce ne sont pas 573 tests unitaires indépendants : c'est un runner maison,
  séquentiel, avec accumulateur global. Quelques groupes sont unitaires
  (compression, URL YouTube, parsing), la majorité sont multi-couches
  (Document + HTML/CSS/layout/JS), et aucun n'est un test browser/QEMU complet.
* Le réseau est simulé par monkeypatch/fake sockets dans quelques scénarios;
  aucun serveur HTTP adversarial local n'exerce réellement DNS/TCP/TLS.
* Sécurité réellement testée : absence des globals QuickJS hôte, budget boucle,
  Secure cookie, rejet d'un Domain complètement étranger, isolation
  localStorage par host/port. **SOP, CORS, preflight, credentials, HttpOnly,
  SameSite, iframe, module CORS, canvas taint et mixed content : zéro test
  protecteur.**

Le runner a une bonne valeur de régression fonctionnelle mais une granularité
et un reporting insuffisants : une exception avant `principal()` peut empêcher
le rapport; pas de JUnit/TAP, tags, durée, seed, timeout externe, couverture ni
inventaire machine-readable.

### Première infrastructure WPT recommandée

Ne pas importer toute WPT. Créer d'abord un adaptateur local qui sert des
fixtures déterministes sur deux ports (`127.0.0.1:A/B`), charge une page avec le
moteur réel, collecte `testharness.js` ou un mini protocole JSON, et écrit :

```text
URL             PASS x  FAIL y  UNSUPPORTED z
DOM/events      PASS x  FAIL y  UNSUPPORTED z
fetch/XHR       PASS x  FAIL y  UNSUPPORTED z
cookies/storage PASS x  FAIL y  UNSUPPORTED z
selectors       PASS x  FAIL y  UNSUPPORTED z
```

Conserver le commit WPT exact, une allowlist de tests (pas une liste de résultats
attendus « tous PASS »), et distinguer FAIL de UNSUPPORTED. Premier lot : URL
constructor/percent encoding, DOM tree mutation, event propagation, CORS
simples/preflight, XHR states, cookie path/HttpOnly/SameSite, storage origin et
selectors. Le runner doit sortir non-zéro uniquement sur régression par rapport
à un baseline versionné, tout en publiant les FAIL connus.

## 7. Pages réelles et régression visuelle

`apercu.py` est une base crédible : il installe un hôte `bo` Pillow, utilise le
même `Document`, layout, peinture et display list, sait charger un fichier
local et produire un PNG déterministe. Il **ne valide pas** QPainter, les
métriques exactes Qt, le framebuffer, l'entrée, ni le transport OS; pour une URL
il substitue `curl` et suit les redirects lui-même.

Infrastructure recommandée, petit chantier autonome :

```text
tools/userland/navigateur/visual/
  fixtures/   # HTML/CSS/images/fonts locales, horloge/animations neutralisées
  references/ # PNG revus humainement, créés une fois
  outputs/    # ignoré par git
  diffs/      # ignoré par git
  compare.py  # dimensions, pixel diff, seuil documenté, image diff
```

Premières fixtures : texte/blocs, cascade/sélecteurs, flex+grid, image locale,
canvas. Fixer largeur/hauteur, polices vendored, DPR, locale et version Pillow;
ne jamais prendre un site dynamique comme golden. L'audit n'a pas ajouté de
goldens : figer des références avec CPython/Pillow non verrouillés aurait donné
une fausse stabilité et mérite une petite review dédiée.

## 8. Performance : instrumenter avant d'optimiser

Il n'existe pas aujourd'hui de télémétrie structurée pour parsing HTML, parsing
CSS, cascade, layout, JS ou paint. Les seules protections/optimisations visibles
sont le budget QuickJS, l'index de règles CSS, le cache HTTP, le préchargement,
la réserve de connexions et l'élagage de peinture hors viewport.

Instrumentation minimale recommandée dans `Document`, activée par
`BO_BROWSER_PROFILE=1`, sans changer les algorithmes : `perf_counter_ns` autour
de HTML, `_regles` (séparer parse feuilles/cascade si possible),
`execute_scripts/tic`, `remet_en_page`, `liste_affichage`; compteurs nodes,
règles, layouts, display-list builds/repaints et opérations peintes. Émettre une
ligne JSON par navigation/frame vers stderr et un résumé à `ferme()`. Ajouter
p50/p95 seulement dans un outil hors moteur.

### Invalidation actuelle démontrée par le flux du code

| Mutation | Travail au battement suivant |
|---|---|
| `style.color = "red"` | `sale=True`, reparsing des règles via `_regles`, cascade/layout **global**, paint global de la vue |
| `style.width = "500px"` | même chemin global (nécessaire au moins pour descendants/frères, mais non borné) |
| `classList.add("foo")` | modification d'attribut, même chemin global |
| `style.transform = "translateX(10px)"` | même layout global, alors qu'une voie paint/composite suffirait théoriquement |

Une série de mutations dans le même tour est coalescée par le booléen `sale`,
ce qui évite un layout par propriété. En revanche `getComputedStyle` d'un
élément sans boîte peut recalculer la cascade de sa lignée et les lectures
géométriques ne forcent pas explicitement un flush synchrone conforme.

Avant toute invalidation fine, mesurer trois fixtures (1k/10k/50k nœuds), et
séparer au minimum `style_dirty`, `layout_dirty`, `paint_dirty`. Une optimisation
de `transform` seule serait locale, mais risque de diverger avec hit testing,
overflow et stacking : seconde review.

## 9. Réseau et robustesse

| Couche | État réel |
|---|---|
| DNS | client UDP IPv4 maison, première réponse A, cache sans TTL, 3 essais × 5 s × serveur; pas AAAA, TCP fallback, CNAME explicite, validation ID/rCode rigoureuse |
| TCP | socket bloquante, timeout global, réserve keep-alive et un retry sur connexion recyclée |
| TLS | Python `ssl`; SNI; vérification CA/hostname si trust store non vide, sinon **CERT_NONE** |
| HTTP/1.1 | `http.client`, corps entièrement bufferisé, redirects max bornés, pool simple |
| HTTP/2 | modules Rust HTTP/2/HPACK présents ailleurs, **non utilisés par le navigateur Python** |
| gzip/deflate | supportés et testés; annonce mensongère rend les octets bruts |
| brotli | code WOFF/codec ailleurs, mais `Accept-Encoding` n'annonce pas `br`; pas de décompression HTTP br |
| cache | ressources `brut` GET 200, max-age/Expires, taille globale; pas ETag/Vary/revalidation/partition par top-level site |
| WebSocket | absent; candidat P1 après sécurité origin |

Erreurs : timeout socket existe mais fetch/XHR timeout/abort n'annulent rien;
connexion keep-alive morte est retentée une fois. Une réponse tronquée dépend du
comportement `http.client` et n'a pas de test dédié. Certificat invalide et
hostname incorrect sont rejetés seulement si un magasin est disponible. La
redirection est bornée, mais l'épuisement retourne la dernière réponse 3xx sans
erreur explicite. Sur redirect cross-host, des headers applicatifs (dont
`Authorization`) peuvent être repris vers la cible : risque de fuite. Tout le
corps est lu en mémoire; aucune limite de headers/corps, streaming ni
backpressure.

Matrice de tests locale P0/P1 à ajouter avec serveurs contrôlés : socket qui ne
répond pas, fermeture milieu de headers/corps, Content-Length trop grand/petit,
chunk tronqué, TLS CA invalide/hostname faux/expiré, redirect 11 bonds et boucle,
redirect vers autre host avec Authorization, gzip tronqué, brotli annoncé,
slowloris, corps > limite. Ne pas commencer HTTP/3.

## 10. Principaux risques techniques

1. **Exfiltration cross-origin et filesystem** : bloque tout usage hostile.
2. **Single process/UI thread** : disponibilité de tout le navigateur dépend
   d'une page et d'extensions natives C/C++.
3. **TLS fail-open** : navigation silencieusement interceptable selon l'image OS.
4. **Transport bloquant et corps non bornés** : freeze/OOM faciles à distance.
5. **Sémantique Web maison sans oracle WPT** : régressions et faux positifs de
   compatibilité impossibles à quantifier.
6. **Invalidation globale** : coût O(page) à chaque frame animée/mutation.
7. **Persistance globale non quota/partitionnée** : épuisement disque/mémoire,
   tracking et corruption concurrente.
8. **Plusieurs navigateurs/prototypes nommés Nautile/browser** : risque de
   tester ou corriger le mauvais chemin.

## 11. Roadmap proposée

### P0 — avant contenu non fiable

* fermer HTTP(S)->`file://` et mixed active content;
* définir Origin/request context et appliquer SOP+CORS+preflight+credentials à
  fetch/XHR/scripts/modules/images/styles;
* séparer cookies réseau/document, implémenter HttpOnly et tests adversariaux;
* supprimer le fail-open TLS : image sans CA = erreur explicite, fournir le
  trust store dans l'artefact;
* limites corps/headers/redirects, annulation et watchdog; tests réseau locaux;
* rendre visibles les FAIL sécurité dans CI, même avant correction.

### P1 — mesure, robustesse, isolation minimale

* runner WPT minimal et serveur bi-origin;
* profiler JSON et budgets nœuds/règles/timers/mémoire;
* suite visuelle déterministe `apercu.py`;
* modèle cookies complet (SameSite, host-only, PSL, Path correct);
* WebSocket avec contrôle Origin;
* prototype renderer séparé fondé sur IPC existant, derrière feature flag;
* redirect/Authorization, streaming, brotli HTTP, cache Vary/ETag.

### P2 — compatibilité guidée par résultats

* browsing contexts/iframe et sandbox, event loop/navigation plus conformes;
* workers seulement après isolation/capabilities;
* invalidation style/layout/paint graduelle à partir des profils;
* HTTP/2 réellement branché si les mesures le justifient;
* accessibility et encodages/URL guidés par WPT.

### P3 — maturité

* renderer par site instance, crash recovery et quotas kernel éprouvés;
* partitionnement cache/storage, permissions/CSP/SRI complets;
* couverture WPT élargie et dashboards performance/visuels;
* nettoyage legacy après deux releases et migration documentée.

## Handoff

### Top 5 à traiter ensuite

1. **SOP/CORS/credentials inexistants** — fichiers : `moteur/js.py`,
   `prelude.js`, `reseau.py`, futur `web_security.py`. Reproduction : SEC-01,
   serveur local sur deux ports; assertions fetch et XHR.
2. **Accès `file://` et mixed content depuis le Web** — fichiers : `js.py`,
   chargeurs `images.py`, `prechargement.py`, feuilles/scripts/modules.
   Reproduction : SEC-04, vérifier qu'aucune fonction I/O fictive n'est appelée.
3. **Cookies HttpOnly/SameSite/host-only** — fichiers : `stockage.py`, `js.py`,
   `reseau.py`. Reproduction : SEC-02 puis requêtes top-level/cross-site POST.
4. **TLS fail-open et réseau non borné** — fichier : `reseau.py`, packaging CA
   dans `build-navigateur.sh`. Tests : CA invalide, hostname incorrect,
   timeout, corps tronqué/géant, redirect Authorization.
5. **Page hostile bloque le processus unique** — fichiers : `bojs.cpp`,
   `js.py`, `__init__.py`, `hote.cpp`; dépendances kernel dans
   `process.rs`, `syscall.rs`, `vmm.rs`, `fd.rs`. Tests : boucle, OOM JS, 50k
   DOM, 10k timers, relayout permanent, crash volontaire renderer.

### Changements déconseillés immédiatement

Pas de réécriture layout, suppression massive de Nautile/pybrowser, intégration
WPT complète, nouveau framework IPC généraliste, HTTP/3, WebGL/WebRTC, ni ajout
en volume de CSS/APIs. Ne pas « corriger » les résultats WPT en les classant
UNSUPPORTED sans règle objective. Ne pas produire des goldens depuis des sites
dynamiques.

### Décisions nécessitant une seconde review

* forme et ownership de `RequestContext` (top-level origin, initiator origin,
  destination, mode, credentials, redirect);
* cookies dans browser process et réseau brokerisé;
* granularité renderer (onglet, origin ou site instance) et protocole IPC;
* format de surface partagée/display list et validation des messages;
* quotas kernel/processus, capability model et politique de crash recovery;
* stratégie PSL/trust store/mises à jour de sécurité;
* baseline WPT et seuils de diff visuel/performance.

Le prochain ingénieur devrait commencer par écrire les quatre tests SEC ci-dessus
sur un serveur local bi-origin **avant** de modifier le transport. L'ordre exigé
reste : reproduction → test rouge → correctif minimal → test vert →
`./tools/userland/test-moteur.sh` complet.
