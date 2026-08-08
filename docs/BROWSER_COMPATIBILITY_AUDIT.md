# Audit de compatibilité fonctionnelle de Bouchaud Browser

**Révision étudiée :** `8f1b872`, 8 août 2026. **Priorité :** compatibilité
réelle (70 %), performance/outillage (20 %), sécurité fondamentale (10 %).

Tout ce que ce document affirme se reproduit :

```bash
./tools/userland/suivi.sh              # tout, avec l'écart depuis la dernière fois
./tools/userland/test-moteur.sh        # 926 vérifications
cd tools/userland/navigateur
python3 serveur_test.py                # le serveur de fixtures, à la main
python3 compatibilite.py --corpus --fixtures
```

---

## Conclusion courte

La phase précédente disait : « le principal écart avec un site moderne n'est
plus une propriété CSS isolée, c'est le contrat comportemental du navigateur
autour de la page. » C'était juste, et cette phase a mesuré puis refermé trois
des quatre trous nommés.

Ce qui a changé : **le foyer existe**, la **plate-forme formulaire** existe,
**`history.pushState` fait ce qu'il annonce**, et **le réseau ne bloque plus le
fil de l'interface**. Le parcours complet d'une page de connexion — ouverture,
clic, frappe, `Tab`, case, soumission, `FormData`, POST, réponse, appel JSON,
mise à jour du DOM, `pushState`, `back`, `popstate` — passe de bout en bout
contre un vrai serveur.

Ce qui n'a pas changé : `<iframe>` n'est toujours pas un contexte de
navigation, WebSocket et IndexedDB n'existent pas, et la mise en page reste la
dépense dominante du moteur.

**Une priorité annoncée a été abandonnée après mesure.** L'event loop devait
être un chantier ; l'ordre observé est déjà celui du Web, à la micro-tâche
près. Le corriger aurait consisté à le casser. C'est le premier bénéfice
concret du compteur : il a évité un chantier, pas seulement guidé un.

---

## 0. Le système de mesure, d'abord

La revue de la PR #140 relevait deux défauts. Les deux sont corrigés, et le
compteur a été refondu autour d'eux.

### A — un identifiant applicatif n'est pas une API navigateur

Tout `X is not defined` était rangé en `api_absente:`. `currentUsr` devenait
donc une lacune du moteur, alors que c'est un défaut de la page — ou, bien plus
souvent, la conséquence d'un bundle qui n'a pas chargé. Une feuille de route
bâtie là-dessus aurait été pleine de noms qu'aucun navigateur ne fournit,
pendant que les vrais manques se seraient perdus dans le bruit.

Un registre de surfaces Web tranche désormais, et quatre catégories remplacent
l'amalgame :

| Catégorie | Ce qu'elle veut dire | Ce qu'on en fait |
|---|---|---|
| `api_absente` | le nom est au registre, le moteur ne le fournit pas | à implémenter |
| `identifiant_inconnu` | la page attend un nom qui n'est d'aucun navigateur | chercher la ressource perdue |
| `methode_absente` | un membre appelé sur `undefined`, ou non appelable | à diagnostiquer |
| `erreur_js` | le reste, groupé par genre | contexte |

Se tromper dans le sens « trop prudent » est sans danger : un manque réel
finira par revenir par un autre chemin — un accesseur du prélude, une méthode
absente, une ressource échouée.

Deux précisions que la mesure a imposées. QuickJS **ne nomme pas** le membre
d'un `x.y()` : le message est un « TypeError: not a function » nu. Ces cas sont
comptés sans nom plutôt qu'affublés d'un nom inventé. En revanche
`cannot read property 'X' of undefined` nomme bien `X`, et c'est la forme la
plus utile de toutes — c'est exactement celle qui tuait le JavaScript de
pypi.org.

### B — toutes les ressources critiques, pas seulement `fetch`

Le compteur ne voyait que `fetch` et `XMLHttpRequest`. Un `<script src>`
principal en 404 vidait la page de tout comportement sans que le rapport n'en
dise un mot. Huit destinations l'alimentent maintenant — `script`, `module`,
`stylesheet`, `image`, `font`, `media`, `fetch`, `xhr` — avec le code HTTP et
l'adresse conservés en exemple.

L'effet est immédiat et instructif : sur `pypi.org/project/requests`, le
rapport passe de « 7 images » à « 9 scripts, 6 fetch, 7 images ». Les neuf
scripts échouaient déjà ; ils étaient simplement invisibles.

### Une troisième catégorie que la revue ne demandait pas : les moignons

Une API présente mais vide est souvent **pire** qu'une absente. La page teste
`if (history.pushState)`, obtient `true`, prend le chemin moderne, et poursuit
avec un navigateur dont l'état ne correspond plus à ce qu'elle croit. Une
absence l'aurait fait retomber sur son plan de secours.

`moignon_appele` compte donc ces appels-là séparément, **sans changer ce que le
moignon rend** — la mesure ne doit pas infléchir ce qu'elle mesure. C'est ce
compteur qui a désigné `history.pushState`, `replaceState`, `element.focus`,
`blur` et `window.scrollTo`, et une vérification garde maintenant la réciproque :
le jour où l'un d'eux redeviendrait vide, elle le dirait.

### Le serveur local de fixtures

L'audit précédent s'est heurté au proxy et son corpus s'est réduit à un
domaine. Cela n'a pas seulement limité la mesure — cela a rendu **intestable**
tout ce qui compte maintenant : une redirection, un 500, un POST, un module
importé par un module, une réponse lente pendant qu'on vérifie que l'interface
répond encore.

`navigateur/serveur_test.py` sert `/json`, `/echo`, `/redirect?to=…&n=…`,
`/status/404`, `/status/500`, `/form/get`, `/form/post`, `/delay/N`,
`/module/main.js` et ses voisins imbriqués, `/cookie/pose`, `/404.js`, plus les
pages témoins du dépôt. Deux origines à la demande, aucune date, aucun
aléatoire : deux exécutions rendent les mêmes octets.

---

## 1. Ce que la mesure a dit, et ce qu'on en a fait

Mesure du 8 août contre le serveur local, avant tout correctif de cette phase.

| Axe | Observé | Décision |
|---|---|---|
| **Event loop** | `sync, promise, microtask, promise-1, promise-2, timer-0, timer-1, raf` — l'ordre du Web, file de micro-tâches vidangée comprise | **abandonné** : rien à corriger |
| **Fetch** | `ok`, `status`, `url`, `headers.get`, `json()`, `text()`, 404, 500, redirections, POST JSON, POST FormData : tout fonctionne | complété : `AbortController` |
| **History / SPA** | `pushState`/`replaceState` moignons, `location` figée, `popstate` jamais émis | **implémenté** |
| **Formulaires** | `activeElement`, `form.elements/action/method/submit`, `select`, `required`, `label.htmlFor` : tout absent | **implémenté** |
| **Réseau lent** | **0** battement de minuterie pendant un `fetch` d'une seconde | **implémenté** |

### Pourquoi l'event loop a été abandonné

C'est la décision la plus utile de la phase, et elle consiste à ne rien faire.
La fixture `tests/pages/event-loop.html` enregistre l'ordre réel ; il coïncide
avec celui d'un navigateur, y compris pour une micro-tâche qui en pose une
autre — les deux passent avant la première minuterie, ce qui prouve que la file
est vidangée et non consommée d'un cran.

La réserve honnête : `requestAnimationFrame` reste adossé aux minuteries. La
séquence *écriture de style → lecture de géométrie dans un `rAF`* n'est donc pas
garantie conforme, et ce cas-là n'est pas couvert par la fixture. Le jour où une
page réelle en souffrira, la mesure le montrera — pas avant.

---

## 2. Le foyer, désormais une primitive

Le foyer appartient au **document**, pas au JavaScript ni au chrome. Les deux le
déplacent — un clic réel d'un côté, un `element.focus()` de l'autre — et les
deux doivent lire le même. Le mettre ailleurs aurait donné deux vérités, donc un
`document.activeElement` qui contredit ce que l'utilisateur voit surligné.

Ce qui fonctionne et est vérifié : `document.activeElement` (qui rend `body`
quand rien n'a le foyer, jamais `null` — du code écrit
`document.activeElement.tagName` sans garde), `focus()`, `blur()`, les
événements `focus`/`blur`/`focusin`/`focusout` dans l'ordre de la norme —
`blur` de l'ancien **avant** `focus` du nouveau —, `Tab` et `Maj+Tab` qui
suivent l'ordre du document, sautent les éléments désactivés et font le tour, et
`:focus` qui peint réellement.

Ce qui manque : l'ordre par `tabindex` positif. Il est rare, souvent déconseillé,
et le simuler à moitié serait pire que de suivre l'ordre du document — qui est
ce que font les pages bien construites.

---

## 3. Formulaires

| Surface | État |
|---|---|
| `form.elements`, indexable par nom | fait |
| `form.action` résolue en absolu, `form.method` en minuscules | fait |
| `form.submit()` / `requestSubmit()` — qui diffèrent par l'événement | fait |
| soumission réelle GET (la requête est **remplacée**) et POST | fait |
| `input.form`, `label.htmlFor`, `label.control` | fait |
| `required`, `checkValidity`, `validity.valueMissing` | fait |
| `select()`, `setSelectionRange`, `selectionStart`/`End` | fait |
| `value` de `select`/`textarea`/`option`, `FormData`, `URLSearchParams` | déjà fait |
| validation HTML5 complète (motifs, bornes, types) | non, et volontairement |
| curseur visible, sélection rendue | non |

Un bug trouvé en écrivant les vérifications, qui n'aurait été visible d'aucune
autre façon : le `checkValidity` du formulaire, posé sur `Element.prototype`,
écrasait celui des champs. Chaque champ répondait donc comme un formulaire
vide — c'est-à-dire toujours valide.

---

## 4. History et navigation SPA

Le document tient son historique de session : une liste d'entrées
`(adresse, état, titre)` et un index. `pushState` ajoute et **tronque ce qui
suit** — c'est ce qui fait qu'après un retour puis une nouvelle navigation on ne
peut plus avancer. `replaceState` écrase. `back()`/`forward()`/`go()` déplacent
l'index, changent l'adresse du document et émettent `popstate` avec l'état
mémorisé, sans aucune requête.

Sortir de la plage reste une **vraie** navigation, déléguée au navigateur : les
deux historiques ne se contredisent pas, ils se relaient.

`location.hash` en écriture passe par le même chemin et émet `hashchange`. Le
confondre avec une navigation rechargeait la page à chaque ancre cliquée.

---

## 5. Le réseau ne bloque plus l'interface

`fetch` rendait une promesse mais partait sur le fil de l'interface. Mesure sur
une réponse d'une seconde :

| | battements de minuterie pendant le vol | réponse |
|---|---|---|
| avant | **0** | 1009 ms |
| après | **48** | 1029 ms |

Un groupe de quatre fils dépose les réponses dans la file que `tic()` vidait
déjà — l'architecture était bonne, il manquait le fil. Quatre, parce que c'est
l'ordre de grandeur du budget de connexions par hôte d'un navigateur, et
qu'au-delà le goulot devient la réserve de connexions, qui porte son propre
verrou. Le mode synchrone de `XMLHttpRequest` reste synchrone : c'est ce qu'il
promet.

`AbortController` s'appuie dessus. La requête n'est pas interrompue au milieu —
on ne va pas fermer une prise depuis un autre fil — mais sa réponse ne sera
jamais livrée et la promesse est rejetée avec un `AbortError`. Du point de vue
de la page, c'est exactement une annulation.

Ce qui reste ouvert : le corps est toujours lu entièrement en mémoire, il n'y a
ni `ReadableStream` ni borne de taille, et le préchargement des sous-ressources
reste sur son propre `ThreadPoolExecutor` indépendant.

---

## 6. iframe

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

## 7. WebSocket

`WebSocket` n'apparaît ni dans `prelude.js` ni dans le pont réseau : il est
absent. Le noyau fournit des sockets TCP clientes, mais il faut encore handshake
HTTP Upgrade, framing, masking, ping/pong, close, limites et API événementielle.

Test local minimal : serveur echo, `open`, texte dans les deux sens, deux
messages ordonnés, ping, close normal, fermeture brutale et timeout. Ajouter au
corpus une fixture « notifications/chat ». WebSocket devient **P1 seulement si**
le nouveau compteur ou l'analyse des erreurs enregistrées montre qu'une page
cible l'instancie; il est néanmoins avant HTTP/3 car il débloque une capacité
applicative, contrairement à une optimisation de transport.

## 8. IndexedDB

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

## 9. CSS réellement impactant

Sans corpus téléchargeable, aucun classement par fréquence n'est publié. Le
code et les anciens rendus prouvent cependant l'impact structurel déjà observé
de flex intrinsic sizing, tables, whitespace inline, inline-block et champs;
ils sont corrigés et couverts.

Manques structurants à rechercher en premier dans les futurs compteurs :
container queries (actuellement transparentes), `display: contents`, aspect
ratio/intrinsic sizing avancé, sticky/overflow imbriqué, rowspan et contrôles de
formulaire. Les effets comme `backdrop-filter`, filtres ou dégradé conique ne
doivent pas devancer un manque layout sans une fréquence/impact observés.

## 10. Fonts et texte

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

## 11. Rendu déterministe et WPT ciblé

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

## 12. Performance visible

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

## 13. Top 20 des causes réelles ou directement démontrables de panne

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


---

## 14. BEFORE / AFTER / REMAINING BLOCKERS

Mesures reproductibles, serveur local pour le comportement et corpus pypi.org
pour les manques agrégés.

### BEFORE → AFTER

| Mesure | Avant | Après | |
|---|---|---|---|
| Vérifications du moteur | 805 | **926** | +121 |
| Contrôles de pixels (Qt réel) | 31 | 31 | = |
| `document.activeElement` | `null` toujours | élément réel, `body` par défaut | ✓ |
| `element.focus()` / `blur()` | moignons vides | foyer réel + événements ordonnés | ✓ |
| `Tab` / `Maj+Tab` | inexistant | ordre du document, saute le désactivé, boucle | ✓ |
| `:focus` peint | jamais | oui | ✓ |
| `form.elements` / `action` / `method` / `submit` | absents | présents et conformes | ✓ |
| Soumission GET / POST réelle | absente | les deux, requête remplacée en GET | ✓ |
| `required` / `checkValidity` | absents | présents | ✓ |
| `label.htmlFor` / `label.control` | absents | présents | ✓ |
| `history.pushState` / `replaceState` | moignons | historique de session complet | ✓ |
| `location.pathname` après `pushState` | figée | suit | ✓ |
| `popstate` | jamais émis | émis avec l'état mémorisé | ✓ |
| Battements de minuterie pendant un `fetch` d'1 s | **0** | **48** | ✓ |
| `AbortController` | absent | annulation réelle, `AbortError` | ✓ |
| Destinations comptées en échec | 2 (`fetch`, `xhr`) | 8 | ✓ |
| Échecs de script visibles sur pypi | 0 (invisibles) | 9 | ✓ |
| `currentUsr` classé API navigateur | oui | non (`identifiant_inconnu`) | ✓ |
| Moignons comptés | catégorie inexistante | 10 surfaces déclarées | ✓ |
| Parcours complet de connexion (15 étapes) | impossible | **passe** | ✓ |
| Temps JavaScript, corpus (le `fetch` bloquant en sortait) | 2 458 ms | 190 ms | −92 % |

Les échecs de ressources passent de 13 à 22 sur le corpus. Ce n'est **pas** une
régression : les neuf scripts en échec échouaient déjà, ils étaient invisibles.
Le proxy de développement bloque `cdn.jsdelivr.net` et `analytics.python.org`.

### REMAINING BLOCKERS

Par ordre de ce que la mesure justifie aujourd'hui.

| # | Blocage | Preuve | Coût estimé |
|---|---|---|---|
| 1 | **Édition clavier réelle dans un champ** — la valeur se pose par script, pas par frappe : pas de `keydown`→insertion, pas de curseur, pas de `Backspace` | le parcours complet doit écrire `input.value` au lieu de taper | moyen ; demande un modèle de curseur et le lien clavier→foyer côté hôte |
| 2 | **Clic réel → foyer** — le foyer se déplace par `focus()` et par `Tab`, pas encore par un clic du chrome | `navigateur.py` ne relie pas encore le clic à `pose_foyer` | faible |
| 3 | **`<iframe>` n'est pas un contexte de navigation** — et il commande SOP | §6 ; zéro occurrence dans `moteur/` | élevé |
| 4 | **Mise en page : 14,7 s cumulées sur le corpus**, contre 3,4 s de CSS et 0,2 s de JS | `suivi.sh`, section Temps | élevé ; la cascade est appelée ~8× par élément |
| 5 | **`cursor`** — 168 déclarations sur 4/5 sites, la propriété fonctionnelle la plus fréquente | rapport CSS | faible côté moteur, petit côté `hote.cpp` |
| 6 | **`fit-content()`, `min()`, `max()`, `clamp()`** — 288 rejets ; une longueur non comprise vaut zéro, donc un bloc effondré | rapport CSS | faible à moyen |
| 7 | **Pointage du contenu positionné hors de son parent** — tout menu ou modale sorti de son conteneur est incliquable | trouvé en écrivant le test de `pointer-events` | moyen |
| 8 | **`requestAnimationFrame` adossé aux minuteries** — la séquence écriture de style → lecture de géométrie n'est pas garantie | non couvert par la fixture ; aucune page réelle ne l'a encore montré | moyen |
| 9 | **WebSocket, IndexedDB** | §7 et §8 ; **zéro demande** sur le corpus atteignable | à ne pas commencer sans mesure |
| 10 | **Corps de réponse non borné, pas de `ReadableStream`** | §5 | moyen |

Ce que la mesure dit de ne **pas** faire maintenant : l'event loop (déjà
conforme), HTTP/2 (le réseau pèse 2,8 s d'un chargement de 11,7 s), SOP/CORS
complet (il n'y a jamais deux contextes de navigation vivants — voir
`BROWSER_COMPATIBILITY_ROADMAP.md` §2.3).

---

## Handoff

Le prochain chantier le mieux justifié est le **blocage 1**, l'édition clavier :
c'est la seule étape du parcours de connexion qui reste simulée par script
plutôt que par une frappe. Les blocages 2 et 5 sont petits et touchent le même
code hôte, ils peuvent voyager avec.

La règle reste : `suivi.sh --strict` avant de pousser, et une fonctionnalité
n'est finie que lorsqu'une fixture la joue, qu'un test comportemental la garde,
que le rapport de compatibilité est propre et qu'aucune mesure n'a reculé.
