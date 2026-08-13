# Feuille de route — compatibilité du navigateur Bouchaud

> **État de ce document.** Sa partie *mesure* et sa partie *plan* restent
> courantes : le WPT n'existe toujours pas, et les outils de suivi décrits ici
> sont ceux qu'on lance aujourd'hui. Ses **inventaires de manques** ont en
> revanche été rattrapés par le travail : `iframe`, WebSocket, Workers,
> IndexedDB, Same-Origin Policy et le renderer séparé existent désormais et sont
> éprouvés. Les passages concernés portent une note ; en cas de doute, la source
> de vérité est `tools/userland/navigateur/tests/jalons.json`, écrit par les
> épreuves, et le `README.md` de la racine.

Ce document remplace l'intuition par la mesure. Chaque affirmation qu'il
contient est reproductible avec les outils du dépôt :

```bash
./tools/userland/suivi.sh                    # tout, avec l'écart depuis la dernière fois
./tools/userland/suivi.sh --rapide           # sans réseau ni Qt, quelques secondes
./tools/userland/suivi.sh --histoire         # la trajectoire
```

`suivi.sh` est le point d'entrée : il enchaîne les trois vérifications
ci-dessous, range leurs nombres au même endroit et les compare à la dernière
exécution de même portée, en disant pour chaque ligne si l'écart est « mieux »
ou « pire ». L'historique est versionné dans
`tools/userland/navigateur/tests/suivi.jsonl`. `--strict` sort en erreur sur un
recul, ce qui en fait une barrière utilisable en intégration continue.

Les outils qu'il appelle restent utilisables seuls :

```bash
./tools/userland/test-moteur.sh                       # 805 vérifications
./tools/userland/verifie-hote.sh                      # 31 contrôles de pixels, Qt réel
cd tools/userland/navigateur
python3 compatibilite.py --corpus --fixtures          # rapport de compatibilité détaillé
python3 apercu.py https://pypi.org/ -o pypi.png       # rendu, sans Qt
```

Répartition de l'effort tenue dans la priorisation ci-dessous : **70 %
compatibilité et fonctionnalités réelles, 20 % performance et outillage,
10 % sécurité fondamentale.**

---

## 1. État actuel

### Ce qui existe

Un seul moteur, un seul chemin de code : `bo-navigateur` (ELF statique, ring 3)
→ `hote.cpp` (Qt5 + CPython embarqué + QuickJS + ffmpeg) → `navigateur.py` →
`moteur/`. Il n'y a pas de second navigateur concurrent dans le dépôt ; le nom
« Nautile » ne subsiste que dans quelques chaînes d'agent utilisateur et dans
des modules noyau sans rapport (`src/net`, `src/gui`).

| Domaine | État vérifié |
|---|---|
| HTML | analyseur tolérant, arbre `Element`/`Texte`, table des balises vides et auto-fermantes |
| CSS — sélecteurs | moteur unique servant la cascade **et** `querySelector` ; spécificité conforme ; `:is`, `:where`, `:not`, `:has`, `nth-*`, attributs, états |
| CSS — mise en page | bloc, en ligne, `inline-block`, flex, grille (dont zones nommées), tableaux, flottants, `position` (dont `sticky` et `fixed`) |
| CSS — décoration | rayons, ombres portées et internes, dégradés linéaires/radiaux/coniques, bordures par côté, `opacity`, `transform` |
| CSS — animation | `@keyframes`, `transition`, pseudo-classes d'état réelles |
| CSS — propriétés logiques | `margin-*`, `padding-*`, `inset`, `border-*`, tailles logiques (ajouté par cette session) |
| Polices | `@font-face`, WOFF (zlib) et WOFF2 (brotli + inversion de la transformation `glyf`/`loca`), `unicode-range` |
| Canvas 2D | chemins, dégradés, ombres, texte, `getImageData`/`putImageData` par rastérisation Qt hors écran |
| Shadow DOM | portée réelle, `:host`, `slot` |
| JavaScript | QuickJS ; DOM, événements, minuteries, promesses, `fetch`, `XMLHttpRequest`, modules ES, `MutationObserver`, `IntersectionObserver`, `ResizeObserver`, `customElements` |
| Réseau | DNS maison, TCP, TLS (fail-closed depuis cette session), HTTP/1.1, gzip/deflate, réserve de connexions, préchargement, cache disque |
| Persistance | témoins (avec `HttpOnly`, chemin RFC 6265, `host-only`), `localStorage` cloisonné par origine, cache HTTP |
| Formulaires | `value` de `select`/`textarea`/`option`, `selectedIndex`, `FormData`, `URLSearchParams` |
| CSS — priorité | `!important`, seconde passe de cascade |
| Média | ffmpeg, H.264, MSE partiel, lecteur YouTube de substitution |

### Ce qui n'existait pas au moment de cet inventaire

> **Rattrapé depuis.** `iframe` comme contexte de navigation, WebSocket et WSS,
> Web Workers, IndexedDB, ainsi que les origines et la Same-Origin Policy
> existent et sont éprouvés — voir `jalons.json`. Le reste de la liste tient
> toujours.

Service Workers, CSP, SRI, Referrer-Policy, permissions, arbre
d'accessibilité, impression, HTTP/2 côté navigateur, `ReadableStream`,
`Blob`/`File`, annulation réelle de `fetch`/XHR, pointage du contenu positionné
hors de son parent.

### Ce qui ne peut pas être vérifié ici

QEMU est absent de l'environnement. **Rien de ce qui suit n'a été observé sous
Bouchaud OS lui-même** : tout est mesuré sur la machine de développement, avec
le moteur Python inchangé, un hôte de substitution Pillow (`apercu.py`) ou l'hôte
Qt réel compilé contre le Qt système (`verifie-hote.sh`). C'est une limite qu'il
faut garder en tête à chaque chiffre de ce document.

---

## 2. Revue de l'audit Codex

Lu intégralement (`docs/history/CODEX_BROWSER_AUDIT.md`, 499 lignes). Chaque constat
ci-dessous a été **revérifié dans le code** avant d'être classé. L'audit est
sérieux : il ne s'est trompé sur aucun fait matériel que j'aie pu tester. Là où
je diverge, c'est sur des priorités, pas sur des faits.

### 2.1 ACCORD

**HTTP(S) → `file://` (SEC-04, SEC-01) — critique, exact.**
Vérifié : `reseau.charge` routait sur `_charge_fichier` dès que l'URL commençait
par `file://`, sans savoir qui demandait, et `_op_requete` faisait
`urllib.parse.urljoin(document.url, url)` — or `urljoin("https://banque.test/a",
"file:///etc/passwd")` rend `'file:///etc/passwd'`. Une page distante lisait donc
le disque par `fetch`, `<img>`, `<script src>`, `<link>` ou `@font-face`.
**Corrigé** (commit « Quatre trous que seule une page hostile aurait montrés »),
avec la distinction que le cahier des charges demandait : la navigation de
l'utilisateur reste libre, la requête initiée par un document ne l'est pas, et
`file://` n'est supprimé nulle part.

**Absence de SOP et de CORS — exact.**
Aucun en-tête `Origin` émis, aucune lecture d'`Access-Control-Allow-Origin`,
aucun pré-vol, `credentials` du `fetch` ignoré : `_recupere` appelle
`reseau.charge` qui joint les témoins de l'hôte cible sans condition. Le
scénario SEC-01 est reproductible tel que décrit.

**Témoins `HttpOnly` — exact.** `_analyse_temoin` ne créait aucun champ
`httponly`, et `_op_temoins` appelait le même `pour()` que l'en-tête réseau.
**Corrigé**, en lecture *et* en écriture : protéger la lecture seule laisserait
fixer la session.

**TLS fail-open — exact et grave.** `_ouvre` posait `check_hostname = False` et
`verify_mode = CERT_NONE` dès que le magasin était vide. **Corrigé** : le
navigateur est fail-closed, cherche un magasin à quatre emplacements plus
`BO_CA_BUNDLE`/`SSL_CERT_FILE`, et refuse HTTPS avec un message explicite sinon.

**Chemin des témoins par `startswith` — exact.** `"/administrateur".startswith("/admin")`
est vrai ; un témoin de `/admin` partait vers une autre application du même hôte.
**Corrigé** selon la RFC 6265 §5.1.4. J'ai ajouté au passage le cas *host-only*
que l'audit mentionne sans le chiffrer : sans attribut `Domain`, un témoin ne
doit pas descendre vers les sous-domaines, et il le faisait.

**Fuite d'en-têtes en redirection cross-host (§9) — exact.** `Authorization` et
`Cookie` de l'hôte de départ étaient repris vers l'hôte d'arrivée. **Corrigé.**

**Canvas sans `origin-clean` — exact.** `_op_rasterise` rend les pixels sans
jamais consulter la provenance des images dessinées. Une page peut donc lire le
contenu d'une image cross-origin. **Non corrigé** : la correction propre demande
un drapeau porté par chaque image du cache et propagé au contexte 2D, ce qui est
une petite architecture, pas un `if`. Rangé P1 (§17).

**Processus unique, fil d'interface unique — exact.** Vérifié dans `hote.cpp` :
`QTimer` de 16 ms sur le fil principal Qt appelant `tic`, et
`BUDGET_MS_DEFAUT = 5000` dans `bojs.cpp`. Un script hostile peut effectivement
figer l'interface cinq secondes. (Voir 2.2 pour ce que je reproche à la
conclusion qu'on en tire.)

**Absence totale de tests protecteurs — exact.** Il n'existait aucune
vérification de SOP, CORS, pré-vol, credentials, `HttpOnly`, `SameSite`, iframe,
CORS des modules, teinte du canvas, contenu mixte. **Il en existe maintenant 34**,
adverses, dans `test_moteur.py` (`politique_ressources`, `tls_ferme`,
`temoins_httponly`, `temoins_document`, `redirection_entetes`).

**Absence d'oracle WPT — exact**, et c'est la remarque la plus structurante de
l'audit : sans oracle externe, « le moteur s'améliore » n'est pas une phrase
vérifiable. Voir §13.

**Invalidation globale — exact.** Chaque mutation lève `sale` et provoque une
remise en page complète.

### 2.2 ACCORD PARTIEL

**« Le budget QuickJS de 5 s bloque l'interface » — le fait est exact, la
conclusion est trompeuse.**
La mesure : sur `https://pypi.org/project/requests/`, page parfaitement bénigne,
**la mise en page seule consomme 5 116 ms** contre 16 ms de JavaScript. Le gel
d'interface n'est pas d'abord un problème de script hostile — c'est le prix
ordinaire d'une mise en page. Abaisser le budget QuickJS ne rendrait pas le
navigateur réactif ; cela n'empêcherait qu'un cas rare, tout en cassant les pages
légitimes qui calculent. La priorité tirée de cette mesure est inverse de celle
que l'audit suggère : **coût de la mise en page d'abord, isolation du moteur de
rendu ensuite, budget JavaScript en dernier.**

**« Instrumenter la performance : `perf_counter_ns` autour de HTML, `_regles`,
`execute_scripts`, `remet_en_page`, `liste_affichage` » — bonne méthode, mais la
liste des suspects rate le vrai coupable.**
Fait (`telemetrie.chrono`, cinq phases). Ce que cela donne :

| Page | HTML | CSS | Mise en page | JS | Peinture |
|---|---|---|---|---|---|
| `pypi.org/project/requests` (2 307 éléments) | 21 ms | 492 ms | **5 116 ms** | 16 ms | 4 ms |
| `pypi.org/` (255 éléments) | 2 ms | **957 ms** | 795 ms | 15 ms | 1 ms |

Puis, en creusant : la mesure de texte par l'hôte n'y est pour rien (8 012 appels,
0,23 s), et `css.applique` est appelé **18 380 fois pour 2 307 éléments** — huit
fois par élément et par chargement — pour 2,74 s. Le coût n'est pas dans
l'invalidation incrémentale que décrit le tableau de l'audit ; il est dans le
**chemin initial**, qui recalcule la cascade du même élément plusieurs fois par
mise en page. C'est là qu'il faut porter le premier effort de performance.

Second fait que l'audit ne relève pas : les feuilles sont analysées **deux fois**
par chargement (une fois avant les scripts, une fois après), ce qui domine les
pages légères.

**« Modèle de témoins complet (SameSite, host-only, PSL, Path correct) » en P1 —
d'accord sur le contenu, pas sur le regroupement.** `Path` et *host-only* étaient
des défauts d'une ligne chacun, corrigeables immédiatement : ils sont faits.
`SameSite` et la PSL sont d'un autre ordre — la PSL demande une liste embarquée
et une politique de mise à jour. Les mettre dans le même lot retardait deux
correctifs gratuits.

**« Ne pas produire de goldens depuis des sites dynamiques » — d'accord sur la
mise en garde, en désaccord avec la conclusion de n'avoir rien construit.**
L'audit conclut qu'il valait mieux ne poser aucune référence. Mais des témoins
**locaux et déterministes**, avec polices embarquées, largeur et DPR figés, n'ont
aucun des défauts qu'il redoute. Onze existent maintenant
(`tools/userland/navigateur/tests/pages/`) et servent déjà de base de mesure.

**« brotli : `Accept-Encoding` ne l'annonce pas » — exact, et incomplet côté
polices.** Le problème réel n'était pas l'absence de décodeur WOFF2 — il existe —
mais une garde dans `police.meilleure_source` qui excluait le format `woff2` de
la liste des formats connus. Écrite avant que le décodeur existe, jamais mise à
jour. Conséquence mesurée : trois familles refusées sur pypi.org, toutes les
polices d'icônes, chaque icône remplacée par un carré. Un mot ajouté à un tuple.
**Corrigé** : huit polices posées, zéro refusée.

### 2.3 DÉSACCORD

**Désaccord 1 — « définir Origin/request context et appliquer SOP + CORS +
pré-vol + credentials à fetch/XHR/scripts/modules/images/styles » ne peut pas
être un P0.**

Trois raisons, dans l'ordre de force :

1. *SOP protège une frontière qui n'existe pas encore.* **(Plus vrai : la SOP,
   les origines et les contextes de navigation existent et sont éprouvés.)**
   La Same-Origin Policy
   règle ce qu'un contexte de navigation peut lire d'un autre. Or le navigateur
   n'a ni `iframe`, ni fenêtre ouverte par script, ni Worker : il n'y a jamais
   deux origines vivantes en même temps dans un même processus. Le seul vecteur
   réel était la lecture *initiée* par un document — `fetch` vers une autre
   origine avec les témoins de la cible, et la lecture du disque. Le second est
   corrigé. Le premier reste, mais il est structurellement moins grave tant
   qu'aucun contenu tiers ne s'exécute dans la page.
2. *L'effet immédiat sur la compatibilité est négatif.* Un CORS correct **bloque**
   des requêtes qui aboutissent aujourd'hui. Sur un navigateur dont le corpus
   mesuré perd déjà 322 déclarations CSS par page et met cinq secondes à poser
   ses boîtes, dépenser le premier trimestre à casser des requêtes qui marchent
   est un mauvais échange.
3. *Le coût est disproportionné.* Pré-vol, modes de requête, modes de
   credentials, état CORS, provenance de la réponse, propagation à travers les
   redirections : c'est un sous-système, pas un correctif. L'audit lui-même le
   range dans « décisions nécessitant une seconde revue ».

Ce que je retiens de sa recommandation, et qui est fait : **le vocabulaire**.
`moteur/securite.py` introduit provenance, origine, destination et un objet
`Requete` prévu pour accueillir `mode` et `credentials` sans déplacer aucun
appelant. L'architecture est décrite en §15. Le sous-système complet est P2,
conditionné à l'arrivée d'un modèle de contexte de navigation — parce que faire
SOP avant les iframes, c'est poser une serrure sur un mur sans porte.

**Désaccord 2 — HTTP/2 n'a pas sa place en P2.**
L'audit le range en « P2 — si les mesures le justifient ». Les mesures ne le
justifient pas : sur la page la plus lourde du corpus, le réseau représente
2,8 s d'un chargement de 11,7 s, et la mise en page 5,1 s. Le multiplexage HTTP/2
attaquerait la troisième cause de lenteur en partant de la plus grosse
complexité. Rangé P3, après que la mise en page soit descendue sous la seconde.

**Désaccord 3 — le prototype de moteur de rendu séparé ne peut pas être en P1.**
*(Tranché depuis : le travail noyau a bien été fait d'abord — mémoire partagée,
IPC avec contre-pression, `RLIMIT_AS`, classes d'ordonnancement — et le renderer
séparé est venu ensuite. Il n'est plus un prototype : c'est le chemin par défaut
du navigateur. Voir `BROWSER_ISOLATION.md`.)*
L'audit le propose « fondé sur l'IPC existant, derrière un drapeau ». Mais les
primitives dont il dépend ne sont pas listées comme acquises : mémoire partagée
entre processus, cycle de vie et terminaison forcée d'un processus fils, quotas,
canal bidirectionnel fiable. Un prototype de moteur de rendu qui découvre ces
manques en chemin coûtera plus cher que le travail noyau fait d'abord et
sciemment. §16 nomme chaque primitive requise. Le tout est P3.

**Désaccord 4 — « plusieurs navigateurs/prototypes nommés Nautile/browser :
risque de tester le mauvais chemin ».**
Vérifié : il n'y a qu'un moteur, sous `tools/userland/navigateur/moteur/`. Les
occurrences de « Nautile » sont des chaînes d'agent utilisateur et des modules
noyau sans rapport avec le navigateur. Le risque existe, mais il est ailleurs et
il est réel : `tools/userland/build-test-moteur/moteur/` est une **copie** que le
lanceur de tests écrase à chaque exécution. Éditer cette copie par mégarde fait
disparaître le travail sans le moindre message — c'est arrivé pendant cette
session, et `compatibilite.py` place délibérément cet atelier en **queue** de
`sys.path` pour ne jamais mesurer le code d'hier.

---

## 3. Corpus de sites réels

### La contrainte, dite franchement

Le proxy de l'environnement de développement refuse tout hôte hors liste. Mesuré
au `curl` :

| Hôte | Résultat |
|---|---|
| pypi.org, files.pythonhosted.org, api.github.com, raw.githubusercontent.com | 200 |
| github.com | 403 (page d'erreur, pas le site) |
| example.com, fr.wikipedia.org, developer.mozilla.org, stackoverflow.com, news.ycombinator.com, duckduckgo.com, docs.python.org, www.gnu.org | `CONNECT tunnel failed, 403` |

Mesurer ces huit-là donnerait huit pages vides et un rapport qui aurait l'air
d'un rapport. Le corpus se limite donc à ce qui répond vraiment, et
`compatibilite.py --tous` réessaie les hôtes bloqués sur une machine sans ce
proxy. C'est une limite de l'environnement, pas du navigateur — mais c'est une
limite qu'il faut lever avant de prétendre mesurer « le Web ».

### Corpus retenu

| Page | Ce qu'elle apporte |
|---|---|
| `pypi.org/` | formulaire de recherche, grille, en-tête collé, polices d'icônes |
| `pypi.org/project/requests/` | 2 307 éléments, onglets, barre latérale, 6 scripts, 10 feuilles |
| `pypi.org/help/` | document long, ancres, table des matières |
| `pypi.org/sponsors/` | beaucoup d'images et de logos, cartes |
| `files.pythonhosted.org/` | page minimale, index servi tel quel |

Complété par **onze témoins locaux déterministes** — `article`, `navbar`,
`cards`, `login-form`, `table`, `dropdown`, `modal`, `sticky-header`, `flex`,
`grid`, `responsive` — dont la mesure ne dépend ni du réseau ni de l'heure.

### Observations par axe (`pypi.org/project/requests/`, après les correctifs)

| Axe | Observation |
|---|---|
| RESEAU | code 200, 7 ressources non chargées (images), 10,1 s de chargement |
| HTML | 6 074 nœuds, 2 307 éléments, 45 balises distinctes, 6 scripts, 10 feuilles |
| CSS | 4 096 règles, 306 déclarations ignorées, 33 propriétés distinctes, 72 valeurs rejetées, 34 sélecteurs non compilés |
| MISE_EN_PAGE | 1 074 boîtes, 11 574 px de haut, 189 opérations peintes, 1 boîte effondrée |
| TEMPS | mise en page 5 116 ms, CSS 492 ms, HTML 21 ms, JS 16 ms, peinture 4 ms |
| POLICES | 8 posées, 0 refusée |
| IMAGES | 183 `<img>` pour 17 adresses distinctes ; 3 chargées, 7 échouées (hôte bloqué) |
| JAVASCRIPT | 0 erreur, 77 863 appels moteur |
| FORMULAIRES | 3 formulaires, 3 champs, 3 listes, 27 boutons |
| NAVIGATION | 288 liens |
| INTERACTIONS | 36 écouteurs posés (`click:16, keydown:8, change:3, keyup:2`) |
| STOCKAGE | `localStorage` et témoins consultés une fois chacun |

### Une mesure qu'il a fallu corriger

La première version de cet axe rapportait « 183 images, 1 peinte » et j'en avais
fait un P0. C'était faux, et de deux façons à la fois. Les opérations peintes
sont **élaguées au viewport** : une page de 11 574 px vue par une fenêtre de
900 px n'en montre qu'une poignée, quel que soit le nombre d'images chargées. Et
183 balises `<img>` ne font que **17 adresses distinctes**, dont 15 sur
`pypi-camo.freetls.fastly.net`, hôte que le proxy bloque. Le compte réel est donc
3 chargées sur 3 joignables, 7 refusées par le réseau : **le navigateur se
comporte correctement, il n'y a pas de défaut d'images.**

La mesure a été refaite au bon endroit — un compteur dans `images.charge` plutôt
qu'un décompte d'opérations peintes — et le P0 retiré. C'est le genre d'erreur
qu'une métrique mal posée produit sans bruit, et la raison pour laquelle chaque
chiffre de ce document doit rester reproductible.

Reste un chiffre qui, lui, tenait : **zéro écouteur d'événement posé** alors que
la page charge six scripts. Il a mené à six défauts réels — §17, P0-2 — et vaut
maintenant 36.

---

## 4. APIs Web manquantes

La mesure est instrumentée : chaque nom absent de la plate-forme porte un
accesseur qui note l'accès et **rend `undefined`**. Rendre autre chose ferait
réussir `if (window.WebSocket)` et changerait le chemin pris par la page — la
mesure fausserait ce qu'elle mesure.

Sur le corpus atteignable, **aucune API surveillée n'a été demandée** : les
scripts de pypi.org meurent avant d'y arriver, sur
`TypeError: cannot read property 'includes' of undefined`, quatre fois sur cinq
pages. C'est un résultat en soi : **la première cause d'inertie du JavaScript
n'est pas une API manquante, c'est une erreur précoce dans du code qui n'aurait
pas dû échouer.** Chercher quelle propriété rend `undefined` là où un navigateur
rend une valeur est plus rentable que d'implémenter WebSocket.

Les 62 noms surveillés (`WebSocket`, `Worker`, `indexedDB`, `caches`,
`BroadcastChannel`, `crypto`, `TextEncoder`, `Intl`, `DOMParser`, `Range`,
`performance`, `matchMedia`, `structuredClone`, `FormData`, `Blob`, …) plus les
membres de `Element.prototype`, `Node.prototype`, `navigator`, `history` et
`location` restent en place : dès qu'un site les touchera, le rapport le dira.

---

## 5. Compatibilité CSS

Après les correctifs de cette session, sur le corpus complet (16 pages) :

| Gravité | Propriétés | Déclarations | Les plus fréquentes |
|---|---|---|---|
| BLOQUANT | 3 | 56 | `table-layout` (40), `border-collapse` (8), `border-spacing` (8) |
| FONCTIONNEL | 4 | 248 | `cursor` (168), `pointer-events` (64), `user-select`, `will-change` |
| TYPOGRAPHIE | 14 | 754 | `font-display` (416), `word-wrap` (96), `direction` (56), `word-break` (56) |
| VISUEL | 6 | 144 | `background-position` (40), `background-size` (32), `outline-color` (24) |
| INCONNU | 7 | 88 | `-webkit-appearance` (32), `-webkit-font-smoothing` (16), `justify-self` (8) |

Valeurs rejetées : `fit-content(40%)` (192 fois), `120px!important` (4 fois — le
`!important` n'est pas retiré avant l'analyse de la longueur).

Sélecteurs non compilés : **entièrement composés de pseudo-éléments
constructeurs** (`::-moz-focus-inner`, `::-webkit-inner-spin-button`, …) que le
moteur a raison d'ignorer, plus `::selection`, `:valid` et `:invalid`. Il n'y a
plus une seule vraie perte à ce poste.

Rappel de l'effet des correctifs, avant → après :

```
propriétés bloquantes ignorées   11 → 3     (152 → 56 déclarations)
sélecteurs non compilés          86 → 34    par page
déclarations ignorées           370 → 322   par page
polices refusées                  3 → 0     sur pypi.org
```

---

## 6. Formulaires

Le témoin `tests/pages/login-form.html` couvre les onze points d'un écran de
connexion : champ texte, champ mot de passe, liste déroulante avec option
présélectionnée, case à cochée, bouton de soumission, événements `input`,
`change`, `focus`, `keydown`, `click`, `submit` avec `preventDefault`, lecture
par `form.elements`, et `FormData`.

Mesure actuelle : la page pose **18 écouteurs** (`input:4, change:4, focus:4,
keydown:4, click:1, submit:1`), les 27 boîtes se placent, les libellés sont liés.
Ce qui n'est pas encore prouvé, faute de scénario de pilotage : la saisie au
clavier, le déplacement du foyer par `Tab`, la soumission réelle et le passage de
`FormData` au réseau. C'est le premier scénario à écrire (§17, P0-3).

Sur le corpus réel : 3 formulaires, 3 champs, 3 listes, 27 boutons sur la fiche
projet de pypi — et **zéro écouteur posé**, parce que le JavaScript du site meurt
avant. Les formulaires ne sont pas cassés ; c'est le script qui ne les atteint
jamais.

---

## 7. Navigation

288 liens détectés et cliquables sur la fiche projet. Ce qui manque et se mesure :
`history.pushState`/`replaceState` (surveillés, jamais appelés sur le corpus),
l'ancre `#fragment` avec défilement, la restauration du défilement au retour, et
la sémantique de `target`. Aucun de ces manques n'est aujourd'hui démontré comme
bloquant par le corpus — d'où leur rang P2.

---

## 8. WebSocket

**Absent.** Impact mesuré sur le corpus atteignable : **nul** — aucune page ne
l'a demandé. Ce n'est pas un argument pour ne jamais le faire, c'est un argument
pour ne pas le faire maintenant : les sites qui en dépendent (messageries,
tableaux de bord temps réel) ne sont de toute façon pas utilisables sans contexte
de navigation ni Workers. Rang P2, après le contrôle d'origine, comme l'audit le
recommande.

---

## 9. IndexedDB

**Absent.** Impact mesuré : **nul** sur le corpus. `localStorage` couvre le
besoin des sites documentaires. IndexedDB devient nécessaire pour les
applications hors ligne, qui supposent des Service Workers, qui supposent une
isolation. Rang P3.

---

## 10. iframe

**Absent au sens fort** : zéro occurrence dans `moteur/`. Un `<iframe>` n'est ni
chargé, ni rendu, ni traité comme un contexte de navigation. Le corpus
atteignable en contient zéro, mais c'est un artefact du corpus : les bandeaux de
consentement, les lecteurs vidéo intégrés et les widgets de paiement en sont
faits.

C'est aussi **la dépendance qui commande SOP** : tant qu'il n'y a pas deux
contextes vivants, il n'y a pas de frontière à défendre. Le bon ordre est donc
`iframe` (contexte de navigation, même sans sandbox) → puis SOP/CORS, et non
l'inverse. Rang P2, en tête de son rang.

---

## 11. Polices

Corrigé pendant cette session ; état actuel sur `pypi.org` : **8 posées, 0
refusée**, polices d'icônes comprises. WOFF, WOFF2 et sfnt direct sont lus ;
`unicode-range` retient la coupe latine. Reste : `font-display` (416
déclarations, sans effet visible tant que le chargement est synchrone),
`font-feature-settings`, `font-variant-numeric` et la synthèse de graisse.

Aucun décodeur maison supplémentaire n'est prévu : la mise en forme complexe
reste du ressort de Qt/HarfBuzz.

---

## 12. Performance

Instrumentée, jamais optimisée à l'aveugle. Les trois faits établis :

1. **La cascade domine la mise en page.** `css.applique` : 18 380 appels pour
   2 307 éléments, 2,74 s sur 5,12 s de mise en page. Le même élément voit sa
   cascade recalculée plusieurs fois par passe.
2. **Les feuilles sont analysées deux fois par chargement** — une fois avant les
   scripts, une fois après. Sur une page légère (`pypi.org/`, 255 éléments) cela
   met l'analyse CSS (957 ms) devant la mise en page (795 ms).
3. **La mesure de texte par l'hôte n'est pas le problème** : 8 012 appels,
   0,23 s, soit 4 % de la mise en page. Contre-intuitif, et cela ferme une piste.

Ce qui n'est **pas** encore mesuré : le coût par mutation en régime établi, la
part du recalcul de style contre celle du placement des boîtes, et le
comportement à 10 000 et 50 000 nœuds. À faire avant toute invalidation fine.

---

## 13. Sous-ensembles WPT

Non commencé, et c'est le manque d'outillage le plus important qui subsiste.
Forme retenue, proche de celle que propose l'audit :

* commit WPT épinglé, liste blanche explicite de répertoires ;
* serveur local **bi-origine** (`127.0.0.1:A` et `127.0.0.1:B`) pour que les
  tests d'origine aient un sens ;
* sortie `URL PASS x FAIL y UNSUPPORTED z`, avec une **règle objective** de
  classement en `UNSUPPORTED` : l'API testée est absente de la plate-forme au
  sens de `telemetrie.NOMS_FENETRE`. Tout le reste est `FAIL`. Sans cette règle
  écrite d'avance, `UNSUPPORTED` devient un tapis sous lequel on glisse les
  échecs — et l'audit a raison de le dire.

Premiers répertoires visés, choisis par la mesure : `css/css-cascade`,
`css/selectors`, `css/css-flexbox`, `css/css-grid`, `html/semantics/forms`.

---

## 14. Sécurité — état des P0

| # | Trou | État |
|---|---|---|
| A | HTTP(S) → `file://` depuis un document | **Corrigé**, 14 vérifications |
| B | TLS fail-open | **Corrigé**, fail-closed, 4 vérifications |
| C | `HttpOnly` exposé à `document.cookie` | **Corrigé**, lecture et écriture, 11 vérifications |
| D | Chemin de témoin par préfixe | **Corrigé** (RFC 6265 §5.1.4) |
| E | Témoin sans `Domain` descendant aux sous-domaines | **Corrigé** (host-only) |
| F | `Authorization`/`Cookie` reportés en redirection cross-host | **Corrigé** |
| G | Canvas sans `origin-clean` | **Ouvert**, P1 |
| H | SOP/CORS | **Ouvert**, P2, voir §2.3 désaccord 1 |
| I | Contenu mixte, CSP, SRI, Referrer-Policy | **Ouvert**, P3 |

---

## 15. Architecture visée pour la politique de sécurité web

Décrite, **non implémentée** — conformément à la consigne. Ce qui existe
aujourd'hui est le premier étage : `moteur/securite.py` (provenance, origine,
destination, objet `Requete`) et le paramètre `document=` de `reseau.charge`.

```
  Document / JavaScript
        │  émet une demande de ressource
        ▼
  RequestContext  ─── ce que la couche transport n'a pas à deviner
        │   origine initiatrice        (tuple schéma/hôte/port, ou opaque)
        │   origine de premier niveau  (pour le cloisonnement du cache)
        │   destination                document | image | style | script |
        │                              font | media | fetch | xhr
        │   mode                       same-origin | cors | no-cors | navigate
        │   mode credentials           omit | same-origin | include
        │   politique de redirection   follow | error | manual
        ▼
  Web Security Policy  ─── le seul endroit qui dit oui ou non
        │   schéma local interdit à un document distant      ← en place
        │   contenu mixte                                     ← à faire
        │   même origine, sinon CORS                          ← à faire
        │   pré-vol si la requête n'est pas simple            ← à faire
        ▼
  Fetch  ─── exécute, puis qualifie la réponse
        │   état CORS de la réponse
        │   provenance : basique | cors | opaque
        │   propagation à travers les redirections
        ▼
  Transport HTTP  ─── ne sait rien de tout cela, et c'est voulu
```

Trois principes que cette forme protège :

1. **Une seule autorité.** Un `if` de sécurité au fond du transport est
   inauditable ; ici tout refus vient de `securite.verifie`.
2. **La provenance voyage avec la réponse**, pas seulement avec la requête :
   c'est ce qui permettra de teinter un canvas, de refuser une lecture de
   `responseText` opaque, et de cloisonner le cache.
3. **Les appelants ne bougent plus.** `mode` et `credentials` s'ajoutent à
   `Requete`, pas aux quinze sites d'appel.

Décisions à trancher avant d'écrire ce code : propriétaire du `RequestContext`
(document ou contexte de navigation), granularité du cloisonnement du cache,
stratégie de liste publique de suffixes, et politique de mise à jour du magasin
de racines.

---

## 16. Architecture multi-processus — prérequis noyau

Non implémentée, non prototypée. Ce que le noyau doit fournir **avant** qu'un
moteur de rendu séparé ait un sens :

| Primitive | Pourquoi | État |
|---|---|---|
| Création de processus avec héritage restreint de descripteurs | le moteur de rendu ne doit pas hériter du disque ni du réseau | à vérifier |
| Canal bidirectionnel fiable (`socketpair` ou tube nommé) | protocole de commandes et de listes d'affichage | à vérifier |
| Mémoire partagée entre processus | passer une surface peinte sans la recopier | `shm-probe.c` existe, non branché |
| Terminaison forcée et détection de mort | un moteur de rendu figé doit être tué, et sa mort constatée | à vérifier |
| Quotas mémoire et CPU par processus | sinon un onglet hostile épuise la machine entière | absent |
| Ordonnancement avec priorité d'interface | l'interface doit rester au-dessus du rendu | partiel |

Granularité proposée, à trancher : une instance par site (et non par onglet ni
par origine), qui est le compromis retenu par les navigateurs réels entre nombre
de processus et surface d'attaque. Le protocole de messages devra être validé
côté navigateur — un moteur de rendu compromis émet des messages arbitraires.

---

## 17. Feuille de route

Chaque entrée porte : **Preuve** — ce qui la justifie dans les mesures de ce
document ; **Sites/tests touchés** ; **Gain de compatibilité attendu** ;
**Complexité** ; **Dépendances**.

### P0 — traité pendant cette session

Les quatre P0 identifiés par la première mesure sont faits. Ce qu'ils ont
révélé en chemin est plus instructif que ce qu'ils annonçaient.

**P0-1. `pointer-events`. — Fait.**
*Preuve* : 248 déclarations FONCTIONNELLES, `pointer-events` 64 fois sur
4 sites. `pointer-events: none` rend la boîte transparente au pointeur ; sans
lui, un calque de modale avalait chaque clic et la page devenait inerte sans
qu'aucune erreur ne le signale.
*Trouvé en chemin* : le test de pointage prenait le **premier** frère couvrant
le point, donc celui que la peinture pose **dessous**. Corrigé en ordre de
peinture. `cursor` (168 déclarations) reste ouvert : il demande un appui de
`hote.cpp` et ne change rien au comportement, seulement à l'apparence du
pointeur. Redescendu P1.

**P0-2. `TypeError: cannot read property 'includes' of undefined`. — Fait.**
*Preuve* : une erreur unique qui tuait tout le script, sur 4 pages sur 5.
*Cause* : `navigator.appVersion` n'existait pas. Puis, une fois passée,
`element.dataset` non plus. Un champ absent de `navigator` ou du DOM ne dégrade
pas — il arrête. Les deux sont implémentés.
*Trouvé en chemin* : la remontée d'ancêtres traversait le pont Python↔JavaScript
une fois par niveau d'arbre. Comme chaque `setAttribute` interroge chaque
`MutationObserver` à portée d'arbre, et qu'un cadre applicatif en pose un sur la
racine, une page faisait **908 430 appels du moteur** pour se construire. Un
seul appel suffit : **77 863** après.
*Mesure, avant → après* : erreurs 1 → 0 ; appels moteur 12 → 165 ; écouteurs
posés 0 → 16 ; accès au stockage 0 → 1.

**P0-3. Scénario de pilotage du témoin `login-form`. — Fait.**
*Trouvé en chemin* : le scénario cherchait sa page témoin dans un dossier que le
lanceur ne copiait pas, ne la trouvait pas, et **sortait en silence**. Il passait
donc toujours. Une fois le décor livré et l'absence rendue fatale, il a échoué —
et révélé deux manques réels : `select.value` rendait la chaîne vide (un
`<select>` porte sa valeur dans l'option choisie, un `<textarea>` dans son texte,
une `<option>` nue dans son libellé), et `FormData` n'existait pas. Les deux sont
implémentés, `URLSearchParams` avec.

**P0-4. `!important`. — Fait, et plus grave qu'annoncé.**
*Preuve* : `<longueur>: 120px!important` rejetée.
*Ce que c'était vraiment* : le drapeau restait collé à la valeur, donc **toute**
déclaration marquée était perdue — pas seulement sa priorité, et pas seulement
les longueurs. La priorité est maintenant honorée par une seconde passe de
cascade.

### P0 — reste ouvert

**P0-5. `cursor`.**
*Preuve* : 168 déclarations sur 4 sites, la propriété FONCTIONNELLE la plus
fréquente du corpus.
*Complexité* : faible côté moteur, petite côté hôte (`QCursor`).
*Dépendances* : `hote.cpp`.

**P0-6. `fit-content()`, `min()`, `max()`, `clamp()` dans les longueurs.**
*Preuve* : `fit-content(40%)` rejetée 288 fois sur 4 sites — la valeur la plus
rejetée du corpus, et une longueur non comprise vaut zéro, donc un bloc effondré.
*Complexité* : faible pour `min`/`max`/`clamp` (arithmétique déjà présente dans
`calc`), moyenne pour `fit-content` (dépend de la largeur intrinsèque).

**P0-7. Le test de pointage n'atteint pas le contenu positionné hors de son
parent.**
*Preuve* : trouvé en écrivant le test de `pointer-events` — un enfant `absolute`
sorti de la boîte de son parent n'est jamais atteint, parce que la descente
s'arrête au premier ancêtre qui ne contient pas le point.
*Gain* : tout menu, toute infobulle, toute modale sortie de son conteneur est
aujourd'hui incliquable.
*Complexité* : moyenne — il faut un ordre de pointage qui suive les contextes
d'empilement plutôt que l'arbre des boîtes.

### P1 — le trimestre

**P1-1. Cascade calculée une fois par élément et par passe.**
*Preuve* : §12 fait 1 — 18 380 appels pour 2 307 éléments, 2,74 s.
*Gain* : performance seulement, mais c'est la performance qui décide de
l'utilisabilité. Objectif : mise en page sous la seconde sur la fiche projet.
*Complexité* : moyenne — mémoriser le style calculé par élément et par passe,
en invalidant sur changement d'état d'interaction.
*Dépendances* : mesures à 1 k / 10 k / 50 k nœuds d'abord.

**P1-2. Analyse des feuilles une seule fois par chargement.**
*Preuve* : §12 fait 2 — 957 ms contre 795 ms sur une page de 255 éléments.
*Complexité* : faible.
*Dépendances* : aucune.

**P1-3. Teinte du canvas (`origin-clean`).**
*Preuve* : §2.1, confirmé dans `_op_rasterise`.
*Gain* : sécurité, pas compatibilité. Peut *retirer* une capacité aux pages qui
lisent des pixels d'images tierces — d'où le rang P1 et non P0.
*Complexité* : moyenne — drapeau par entrée du cache d'images, propagé au
contexte 2D, consulté par `getImageData` et `toDataURL`.
*Dépendances* : `securite.origine` (en place).

**P1-4. Typographie : `letter-spacing`, `word-break`, `overflow-wrap`,
`text-overflow`.**
*Preuve* : 754 déclarations TYPOGRAPHIE, sur 4 sites.
*Gain* : titres et libellés à la bonne largeur, mots longs qui cessent de
déborder de leur colonne.
*Complexité* : moyenne. `letter-spacing` doit traverser l'hôte — mesure **et**
peinture, sinon on mesure dans un état et on dessine dans un autre ; c'est
exactement le défaut qui avait déjà été trouvé sur la famille de police.
*Dépendances* : `hote.cpp`, `apercu.py`, `verifie_hote.cpp`.

**P1-5. Fond : `background-size`, `background-position`, `background-repeat`.**
*Preuve* : 88 déclarations VISUEL sur 4 sites.
*Complexité* : moyenne, côté peinture.

**P1-6. Runner WPT minimal et serveur bi-origine.**
*Preuve* : §13 — sans oracle externe, aucun progrès n'est démontrable.
*Complexité* : moyenne.
*Dépendances* : la règle objective de classement `UNSUPPORTED` doit être écrite
avant la première exécution.

**P1-7. Bornes réseau : taille de corps, taille d'en-têtes, annulation réelle de
`fetch`/XHR, chien de garde.**
*Preuve* : audit §9, revérifié — `abort()` est vide, `timeout` n'est pas appliqué,
le corps est entièrement mis en mémoire.
*Complexité* : moyenne.

### P2 — guidé par les résultats

* **P2-1. `iframe` comme contexte de navigation** (sans sandbox d'abord).
  *Dépendance de* P2-2. Sans lui, SOP protège un mur sans porte.
* **P2-2. SOP + CORS + pré-vol + credentials**, sur l'architecture de §15.
  *Gain de compatibilité attendu : négatif à court terme, positif dès qu'un
  contenu tiers s'exécute.* Complexité forte.
* **P2-3. Tableaux : `table-layout`, `border-collapse`, `border-spacing`**
  (56 déclarations BLOQUANTES restantes).
* **P2-4. `history.pushState` et navigation par fragment.**
* **P2-5. WebSocket, avec contrôle d'origine.** Dépend de P2-2.
* **P2-6. Invalidation graduelle style/mise en page/peinture**, à partir des
  profils de P1-1.
* **P2-7. `SameSite` et liste publique de suffixes.**

### P3 — maturité

* Moteur de rendu par instance de site, avec les prérequis noyau de §16 traités
  en premier et nommément.
* Cloisonnement du cache et du stockage par site de premier niveau.
* CSP, SRI, Referrer-Policy, contenu mixte, permissions.
* Workers, Service Workers, IndexedDB.
* HTTP/2 — **après** que la mise en page soit passée sous la seconde, pas avant.
* Arbre d'accessibilité, impression.

---

## 18. Méthode

L'ordre est le même pour chaque entrée de cette feuille de route, sans exception :

```
échec sur site réel, WPT ou témoin
        → observation reproductible et chiffrée
        → cause racine trouvée dans le code
        → décision de priorité, justifiée par la mesure
        → implémentation
        → test de non-régression
        → nouvelle mesure du corpus
```

Ce que cela exclut : implémenter parce que Chrome l'a. Aucun des cinq
correctifs de cette session n'aurait été choisi par cette logique-là — le
découpage des listes de sélecteurs et la garde WOFF2 périmée ne figurent sur
aucune liste de fonctionnalités.

Ce que cela exclut aussi : améliorer un score WPT en reclassant des `FAIL` en
`UNSUPPORTED`, et figer des références visuelles depuis des sites dynamiques.
