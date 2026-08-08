# Audit de compatibilité fonctionnelle de Bouchaud Browser

**Révision étudiée :** `c84e7da` + instrumentation de cette phase, 8 août
2026. **Priorité :** compatibilité réelle (70 %), performance/outillage (20 %),
sécurité fondamentale (10 %).

## Conclusion courte

Le moteur sait déjà rendre des documents statiques substantiels. Le principal
écart avec un site moderne n'est plus une propriété CSS isolée : c'est le
contrat comportemental du navigateur autour de la page. Les formulaires ne
gèrent ni édition réelle ni soumission, `history.pushState` ne fait rien, les
classes réseau Web (`Headers`, `Request`, `FormData`, `AbortController`) sont
absentes, `<iframe>` n'est pas un browsing context, et il n'existe ni WebSocket
ni IndexedDB. Un bundle moderne peut donc s'arrêter avant même de construire
son interface alors que HTML/CSS sauraient l'afficher.

Cette phase ajoute un compteur **runtime** des API réellement rencontrées : les
`ReferenceError` QuickJS sont agrégées par nom, les méthodes non appelables
séparément et les ressources en échec comptées. `apercu.py` imprime ce rapport.
Contrairement à un scan de texte, deux occurrences situées après une première
exception ne deviennent pas artificiellement deux appels.

## 1. Corpus fonctionnel

### Conditions et honnêteté des résultats

Le corpus prévu est versionné ici comme protocole, mais le chargement distant
n'a pas été possible dans ce conteneur : les huit requêtes `curl` ont reçu
`CONNECT tunnel failed, response 403`. `apercu.py` ne pouvait pas davantage
produire de PNG parce que Pillow n'est pas installé, et son installation a été
refusée par le même proxy. Aucun pourcentage de rendu n'est donc inventé.

Des exécutions antérieures présentes dans l'historique du dépôt ont réellement
utilisé `apercu.py` sur PyPI et des pages réelles : elles ont révélé espaces
inline supprimées, mauvais flex intrinsic sizing, tables, inline-block,
contrôles sans boîte, SVG puis `@font-face`; ces défauts ont depuis reçu des
tests. Elles constituent une preuve utile, mais pas une nouvelle mesure du site
actuel.

| Cible | Réseau cette phase | Moteur/rendu cette phase | Observation exploitable |
|---|---|---|---|
| example.com | BLOQUÉ par proxy avant le moteur | non exécuté | fixture idéale de smoke test statique |
| Wikipedia, article Web browser | BLOQUÉ | non exécuté | HTML article/table/images à mesurer hors proxy |
| PyPI accueil | BLOQUÉ | non exécuté | historique : header flex, champs, SVG et Web fonts ont révélé des défauts réels |
| GitHub login | BLOQUÉ | non exécuté | fixture locale de login nécessaire avant conclusion |
| Stack Overflow questions | BLOQUÉ | non exécuté | JS, navigation et interactions non mesurés |
| DuckDuckGo HTML | BLOQUÉ | non exécuté | bon test de formulaire GET, non mesuré |
| Hacker News | BLOQUÉ | non exécuté | bon baseline HTML léger, non mesuré |
| MDN Web API | BLOQUÉ | non exécuté | article/layout/code, non mesuré |

### Protocole à relancer sur une machine connectée

Pour chaque URL, conserver HTML, ressources et date, lancer
`apercu.py URL -o outputs/site.png`, enregistrer stdout/stderr et le rapport
compatibilité, puis tester manuellement scroll, liens, clavier et formulaire
sous l'OS. La matrice doit utiliser `OK`, `PARTIEL`, `BLOQUÉ(<cause>)` et
`NON TESTÉ`, jamais un pourcentage subjectif.

Colonnes obligatoires : réseau, HTML, CSS, layout, images, fonts, JavaScript,
interactions, formulaires, navigation, scroll, erreurs JS/CSS, API absentes.
Archiver une capture hôte de même viewport uniquement pour le diff; ne pas en
faire une golden durable si le site est dynamique.

## 2. Diagnostic des manques rencontrés

`Contexte.rapport_compatibilite()` rend un dictionnaire trié. Il est alimenté
aux frontières qui voient vraiment l'échec : `execute`, callbacks QuickJS et
fetch/XHR. Exemple attendu :

```text
[compatibilite]
  api_absente:AbortController: 6
  api_absente:URLSearchParams: 8
  methode_absente:inconnue: 3
  ressource_echouee: 2
```

Limite assumée : QuickJS ne donne pas toujours le nom dans `TypeError: not a
function`; le compteur dit alors `inconnue`, plutôt que d'accuser une méthode au
hasard. Les APIs testées par `if ('WebSocket' in window)` mais jamais appelées
ne sont pas comptées. Les erreurs attrapées entièrement par la page ne remontent
pas encore au pont. Prochaine amélioration locale : envelopper `window.onerror`
et `unhandledrejection`, puis ajouter destination et URL aux resource failures.

Le compteur de cette phase prouve `AbortController: 1` sur une fixture qui le
référence deux fois : la première exception arrête le script. La suite complète
atteint maintenant 575 assertions.

### CSS à instrumenter ensuite

Le parseur conserve souvent une déclaration inconnue dans le dictionnaire de
style; c'est le consommateur layout/paint qui l'ignore. Un simple inventaire de
noms « acceptés par le parseur » serait donc trompeur. Ajouter un registre
explicite par consommateur (`layout`, `paint`, `text`, `animation`) puis compter
les déclarations calculées sans consommateur. Séparer :

1. structure (`display`, sizing, position, grid/flex);
2. contrôles/texte/fonts;
3. décoration;
4. animation/compositing.

Même principe pour at-rules et media queries : compter la règle rencontrée et
la raison (`syntaxe`, `feature inconnue`, `contexte non implémenté`). Aujourd'hui
`@container` et `@scope` sont traversées comme transparentes : cela conserve du
style, mais n'est pas une implémentation conforme.

## 3. Couverture concrète des Web APIs

### Navigation / Window

| API | État du code | Impact fonctionnel |
|---|---|---|
| `location` href/assign/replace/reload et parties | PARTIEL, navigation différée | liens/scripts simples utilisables |
| `history.back/forward/go` | PARTIEL, raccordé à l'historique du chrome | navigation classique |
| `pushState/replaceState` | **stubs vides** | routeurs SPA ne changent ni URL ni état |
| `popstate`, `hashchange` | absents | routeurs/navigation intra-page cassés |
| `window.open` | absent | auth/popups et liens ciblés cassés |
| `scrollTo/scrollBy/scroll`, `scrollIntoView` | global minimal/stub; élément stub | scripts sticky/infinite scroll imprécis |
| focus global | stub | modèle de focus absent |

`pushState` ne doit passer P0/P1 qu'après compteur sur le corpus. Sa nature de
stub est toutefois plus dangereuse qu'une absence : les feature detections le
croient fonctionnel et la page continue avec un état faux.

### DOM

Sont implémentés et testés : Node/Element/HTMLElement, parent/enfants/frères,
mutations, attributes, `classList`, HTML intérieur/extérieur (setter outerHTML
absent), `insertAdjacentHTML`, `cloneNode`, `contains`, `closest`, `matches`,
query selectors, rect, événements capture/cible/bubble, shadow DOM partiel.

Manques à mesurer : `dataset`, véritable `DOMTokenList` exposé, collections
live, `replaceWith/before/after`, Range/Selection, focus/activeElement,
`scroll*`, dimensions complètes, iframe document/window. Plusieurs méthodes
présentes sont des stubs silencieux (`focus`, `blur`, `scrollIntoView`) : le
diagnostic doit à terme compter aussi leur appel.

### Network Web APIs

| API | État |
|---|---|
| `fetch` | minimal : URL/options method/body/headers, Response texte/json |
| XHR | minimal; `abort()` vide, timeout ignoré, états incomplets |
| `Headers`, `Request` | absents |
| `Response` global compatible | classe interne non exposée comme surface complète |
| `AbortController/AbortSignal` | absents, désormais détectables au runtime |
| `URL` | objet QuickJS/partiel détourné pour MSE; conformité URL non prouvée |
| `URLSearchParams` | absent |
| `FormData`, `Blob`, `File`, `FileReader` | absents |
| `TextEncoder/TextDecoder` | absents du prelude (à vérifier contre le build QuickJS) |

Ce groupe est le meilleur candidat fonctionnel à mesurer : les bibliothèques
modernes construisent souvent ces objets au bootstrap. Implémenter d'abord
`URLSearchParams`, `Headers` et `AbortController` si le corpus les compte;
`FormData` dépend directement du chantier formulaires.

### Asynchronisme

Promises/microtasks viennent de QuickJS et sont testées. `setTimeout`,
`setInterval`, cancellation, `requestAnimationFrame` et `queueMicrotask`
existent. `requestAnimationFrame` est un timer 16 ms, pas synchronisé à une
phase paint. `requestIdleCallback` est absent. Le réseau qualifié
« asynchrone » est effectué synchroniquement dans `_op_requete`, puis seulement
la livraison est différée : une requête lente fige donc la GUI.

### Stockage / communication

`localStorage` persiste par origin; `sessionStorage` est en mémoire par contexte.
IndexedDB est absent. WebSocket, MessageChannel, `postMessage` et
BroadcastChannel sont absents. Aucun de ces manques n'a une fréquence corpus
mesurée dans cette phase à cause du blocage réseau; ils restent candidats, pas
priorités automatiques.

## 4. Formulaires : état fonctionnel réel

Le layout fabrique désormais une boîte et un texte visible pour input/textarea,
avec styles agent pour text/password/email/search, checkbox/radio/select et
boutons. Le DOM expose `value`, `checked`, `disabled` comme attributs.

Mais un contrôle visible n'est pas encore un contrôle utilisable :

| Fonction | État |
|---|---|
| button/click programmatique | événement click distribué; action par défaut limitée |
| input text/password/email/search | affichage valeur/placeholder; **édition clavier non démontrée** |
| textarea | affichée; modèle valeur/enfants simplifié |
| checkbox/radio | attribut `checked`; groupe radio/action souris non démontrés |
| select/option | layout générique; ouverture et sélection natives absentes |
| label | pas d'association/focus démontré |
| disabled | propriété présente; suppression effective des actions non prouvée |
| autofocus/focus/blur | focus et blur sont stubs |
| submit | pas de `HTMLFormElement`, sérialisation, validation ni navigation démontrées |
| FormData/validation | absents |
| input/change/submit/focus/blur | système Event générique, mais actions natives qui les déclenchent manquent |
| keydown/keyup | événement hôte générique possible; édition/target focus absents |

**Fixture P0 proposée :** login déterministe avec labels, email, password,
checkbox « remember », bouton submit, validation required, sérialisation POST,
serveur local qui reflète les champs, Tab/Shift-Tab, Enter, focus/blur,
input/change/submit et retour d'erreur. Tant qu'elle ne passe pas, GitHub login
ne peut pas être qualifié « utilisable » même si sa capture est correcte.

## 5. iframe

Le parseur conserve `<iframe>` comme un élément, mais aucun Document enfant,
viewport, navigation, `contentWindow/contentDocument`, chargement `src`, resize
ou canal parent/enfant n'est créé. Un embed est donc au mieux une boîte vide.

Tests locaux à écrire par ordre : (1) `srcdoc` texte fixe et taille CSS; (2)
page locale `src`; (3) scroll/resize; (4) parent lit le titre enfant; (5)
`postMessage`; (6) embed vidéo. Pour compatibilité, un browsing context local
minimal peut précéder sandbox/CSP, mais il doit être explicitement réservé aux
fixtures/local tant que SOP n'est pas reviewée. Mesurer les `<iframe>` du corpus
avant P1; GitHub login ou Wikipedia peuvent ne pas en dépendre, les embeds et
auth fédérées oui.

## 6. WebSocket

`WebSocket` n'apparaît ni dans `prelude.js` ni dans le pont réseau : il est
absent. Le noyau fournit des sockets TCP clientes, mais il faut encore handshake
HTTP Upgrade, framing, masking, ping/pong, close, limites et API événementielle.

Test local minimal : serveur echo, `open`, texte dans les deux sens, deux
messages ordonnés, ping, close normal, fermeture brutale et timeout. Ajouter au
corpus une fixture « notifications/chat ». WebSocket devient **P1 seulement si**
le nouveau compteur ou l'analyse des erreurs enregistrées montre qu'une page
cible l'instancie; il est néanmoins avant HTTP/3 car il débloque une capacité
applicative, contrairement à une optimisation de transport.

## 7. IndexedDB

`indexedDB` est absent. Aucun résultat de cette phase ne permet d'affirmer
qu'une des huit pages échoue uniquement pour cela. Le diagnostic runtime saura
maintenant compter un `ReferenceError`, sauf si une bibliothèque fait une
feature detection puis choisit un fallback.

Implémentation progressive si les mesures le justifient :

1. API asynchrone `open`, versions/upgradeneeded, database/object store simple;
2. `get/put/delete/clear`, clés string/nombre, transaction séquentielle;
3. index/cursor; ensuite seulement structured clone étendu et concurrence.

Le stockage doit être par origin, quota et atomique. Un dictionnaire synchrone
déguisé en IndexedDB ferait passer la détection puis casserait les transactions,
donc serait pire qu'une absence explicite.

## 8. CSS réellement impactant

Sans corpus téléchargeable, aucun classement par fréquence n'est publié. Le
code et les anciens rendus prouvent cependant l'impact structurel déjà observé
de flex intrinsic sizing, tables, whitespace inline, inline-block et champs;
ils sont corrigés et couverts.

Manques structurants à rechercher en premier dans les futurs compteurs :
container queries (actuellement transparentes), `display: contents`, aspect
ratio/intrinsic sizing avancé, sticky/overflow imbriqué, rowspan et contrôles de
formulaire. Les effets comme `backdrop-filter`, filtres ou dégradé conique ne
doivent pas devancer un manque layout sans une fréquence/impact observés.

## 9. Fonts et texte

WOFF est réellement décompressé vers sfnt. WOFF2 est reconnu puis refusé; un
journal explicite indique « WOFF2 seul ». Il faut compter ces messages par page
pour mesurer l'impact. Beaucoup de feuilles offrent WOFF2 puis WOFF/TTF : le
moteur choisit la première source lisible, donc la présence de `.woff2` seule ne
prouve pas un échec.

L'hôte Qt mesure et rend le texte et doit rester la voie vers FreeType/HarfBuzz;
ne pas écrire de shaping maison. Le moteur distingue famille et gras; l'italique
est transporté pour les Web fonts mais le fallback embarqué de `apercu.py`
utilise actuellement des choix Mono comme substitut italique, donc le diff peut
être trompeur. À tester : poids 100–900, italic/oblique, icon font PUA, emoji
couleur, arabe/Devanagari, ligatures, bidi, combining marks et fallback par
glyphe. Porter le décodeur WOFF2 standard (`woff2_decompress`/Brotli) si au
moins plusieurs pages n'offrent aucun fallback lisible.

## 10. Rendu déterministe et WPT ciblé

### Fixtures visuelles

Créer après disponibilité Pillow/Chromium : navbar flex, cards, login form,
table avec colspan/rowspan, modal, dropdown, responsive breakpoints, sticky
header, grid gallery et article. Répertoires : `fixtures/`, `references/`,
`outputs/`, `diffs/`. Fixer 1280×900 et 390×844, fontes vendored, animations et
horloge. Référence hôte Chromium; sortie Bouchaud par `apercu.py`; diff pixel et
bounding boxes. Les outputs/diffs ne sont pas versionnés.

### WPT à rendement direct

Premier runner : sous-ensembles URL, DOM mutations, events, forms, selectors,
flex, grid, fetch et Web Storage, avec serveur local. Publier PASS/FAIL/
UNSUPPORTED par domaine et causes regroupées. Ne pas importer un score global
ni reclasser les FAIL. Priorité initiale : forms + events, puis URL/DOM dont
dépendent les frameworks.

## 11. Performance visible

* fetch/XHR bloque pendant toute l'I/O malgré callback différé : freeze visible;
* toute mutation DOM/style pose `sale` et déclenche cascade/layout global au
  battement; animations actives relayoutent à chaque frame;
* DOM, timers, corps réseau et display list n'ont pas de budgets fonctionnels;
* peinture élague hors viewport et les mutations d'un tour sont coalescées;
* index CSS évite le scan exhaustif des règles, gain historique mesuré ×60.

Ajouter des timings `load/network`, HTML, feuilles/CSS, scripts, layout, display
list et paint autour de trois tailles de fixture. Mesurer p95 interaction et
frame >16/50/100 ms, pas seulement temps total. Ne pas réécrire l'invalidation
avant d'avoir ces traces.

## 12. Top 20 des causes réelles ou directement démontrables de panne

Classement par impact fonctionnel probable; **M** signifie à mesurer dans le
corpus avant promotion P0/P1, **D** défaut directement visible dans le code ou
une ancienne exécution réelle.

1. **D** contrôles sans édition/focus/soumission native;
2. **D** fetch/XHR réseau synchrone bloque la GUI;
3. **D** `FormData` absent;
4. **D** `AbortController` absent et maintenant mesurable;
5. **D** `URLSearchParams`/Headers/Request absents;
6. **D** `history.pushState/replaceState` stubs silencieux;
7. **D** popstate/hashchange absents;
8. **D** iframe sans browsing context;
9. **M** WebSocket absent;
10. **M** IndexedDB absent;
11. **D** focus/blur/scrollIntoView stubs;
12. **D** aucun modèle de valeur/sélection pour select/option/radio;
13. **D** WOFF2 seul non décodable;
14. **M** dataset absent;
15. **M** Blob/File/FileReader/TextEncoder manquants;
16. **D** méthodes XHR abort/timeout sans effet;
17. **D** layout global à chaque mutation/animation;
18. **M** container queries traitées comme transparentes;
19. **D** réponses/corps entièrement bufferisés et non bornés;
20. **D** erreurs de méthodes parfois anonymes dans le diagnostic.

Cette liste n'affirme pas que WebSocket ou IndexedDB cassent les sites du corpus
avant mesure. Elle sépare précisément les candidats des blocages prouvés.

## 13. Roadmap conditionnée par les mesures

### P0 — utilisabilité de base démontrée

* fixture login complète : focus, édition text/password/email/search, checkbox,
  label, clavier, input/change/submit, validation simple, FormData et GET/POST;
* réseau réellement asynchrone ou worker afin qu'un formulaire ne fige pas la
  fenêtre;
* instrumentation corpus reproductible (snapshots + diagnostics + timings);
* conserver les trois correctifs sécurité locaux séparés : Web→`file://`
  interdit, TLS fail-closed, HttpOnly caché au JS.

### P1 — seulement après occurrence corpus

* URL/URLSearchParams, Headers/Request/Response et AbortController;
* history state + popstate/hashchange pour les SPA observées;
* WebSocket si au moins une interaction cible est bloquée;
* iframe minimal si embeds/auth du corpus sont bloqués;
* WOFF2 standard si plusieurs pages n'ont pas de fallback;
* IndexedDB minimal si une application cible ne possède pas de fallback.

### P2 — compatibilité importante non bloquante

* Blob/File/FileReader/TextEncoder, dataset/DOMTokenList complet;
* iframe resize/postMessage puis isolation reviewée;
* IndexedDB index/cursor et WebSocket robustesse avancée;
* invalidation style/layout/paint graduelle guidée par traces;
* contrôles forms avancés, Selection/Range, accessibilité.

### P3 — finition et long terme

* effets CSS fréquents mais décoratifs, animation/compositing fin;
* APIs de communication rares d'après corpus;
* shaping/emoji avancés via Qt/HarfBuzz;
* élargissement WPT et corpus, jamais HTTP/3 avant les blocages fonctionnels.

## Handoff

Relancer d'abord le corpus dans un environnement avec réseau et Pillow, sans
modifier le moteur, puis joindre les huit rapports. La première décision est
forms vs bootstrap JS APIs, déterminée par ces rapports. Ensuite : fixture login
rouge, correctif minimal, suite 575 assertions, test browser sous OS. Garder les
P0 sécurité de l'audit précédent documentés, mais faire reviewer SOP/CORS à part
pour ne pas transformer ce chantier de compatibilité en refonte de sécurité.
