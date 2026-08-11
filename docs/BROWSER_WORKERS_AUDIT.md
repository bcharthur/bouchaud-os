# Web Workers : ce qu'il faudrait, et pourquoi c'est le bon prochain client

*Audit, pas implementation. Rien n'est ecrit ; ce document dit ce que coute
`new Worker("worker.js")` avec le moteur tel qu'il est aujourd'hui.*

## Pourquoi maintenant

Un `Worker` est la version legere du probleme que le renderer separe pose en
grand : **deux mondes JavaScript qui vivent en meme temps et ne se parlent que
par messages**. Il partage avec lui l'essentiel des questions — cycle de vie,
isolation, ordonnancement, `postMessage` — et n'en pose aucune des plus
difficiles : pas de mise en page a transporter, pas de surface partagee, pas de
liste d'affichage a encoder.

C'est donc un bon premier client de ce qui vient d'etre construit. Ce qu'un
`Worker` apprendrait sur la duree de vie d'un contexte, sur la livraison de
messages entre deux mondes et sur l'effet des classes d'ordonnancement
s'appliquerait directement au renderer, pour une fraction du travail.

## Ce qui existe deja et servirait tel quel

| Brique | Etat | Ce qu'un Worker en ferait |
|---|---|---|
| Contextes multiples | fait (`contexte.py`) | Un `Worker` est un contexte de plus, sans document. |
| `postMessage` structure | fait (`js.py`) | Le meme chemin : file de messages vidangee par `tic()`, jamais au milieu d'un script. |
| Cycle de vie et fermeture | fait (`Contexte.ferme`) | `terminate()` est exactement `ferme()` : contexte QuickJS detruit, requetes abandonnees, connexions fermees, pools arretes. |
| Reseau asynchrone | fait | `fetch` dans un worker passe par le meme pool. |
| Minuteries | fait | `setTimeout` existe deja par contexte. |
| IndexedDB | fait, avec une reserve | Le service est par origine, donc partageable ; les transactions sont par contexte JS. |
| Classes d'ordonnancement | fait (noyau) | Un worker serait `Normale`, l'interface `Interactive`. |

## Ce qu'il faudrait ecrire

### 1. Un runtime QuickJS par worker — et le fil qui va avec

Aujourd'hui, `bojs.cree()` fabrique un contexte, et tous vivent sur le fil de
l'interface. Un worker doit tourner **ailleurs**, sans quoi son calcul figerait
la page — ce qui reviendrait a ne pas l'avoir ecrit.

La difficulte n'est pas de creer le contexte mais de garantir qu'aucun objet
n'est partage entre les deux. QuickJS n'est pas reentrant : deux fils dans le
meme `JSRuntime` corrompent le tas. Il faut donc un `JSRuntime` distinct, pas
seulement un `JSContext` — et verifier que le pont Python/JS n'attrape pas de
reference au contexte du parent.

**Le vrai risque est la** : un pont qui fonctionne par hasard tant que les deux
fils ne s'entrelacent pas, et qui corrompt la memoire sous charge. C'est le
genre de defaut qui se manifeste une fois sur mille, dans une trace qui ne
designe pas sa cause.

### 2. Un `postMessage` qui traverse un fil

Le mecanisme existe mais suppose les deux bouts sur le meme fil. Entre deux
fils il faut une file protegee — la meme discipline que `_verrou_reponses` —
et une regle claire : **les donnees sont copiees, jamais partagees**. Un objet
transmis par reference donnerait deux fils sur le meme tas QuickJS, c'est-a-dire
exactement ce qu'on cherche a eviter.

Le clonage structure n'a pas besoin d'etre complet : passer par JSON couvre les
objets, tableaux, nombres et chaines, ce qui est l'immense majorite des usages.
Il faut dire ce qui n'est pas couvert — `Map`, `Set`, `ArrayBuffer`,
references circulaires — plutot que de les laisser echouer en silence.

### 3. Une surface globale reduite

Un worker n'a ni `document`, ni `window`, ni DOM. Le prelude suppose partout
qu'ils existent. Il faudrait le decouper en deux : ce qui vaut pour tout
contexte (minuteries, `fetch`, `console`, `postMessage`, `crypto`) et ce qui
n'appartient qu'a un document.

C'est le chantier le plus long, et le moins interessant : trois mille lignes a
partager en deux. Mais c'est aussi celui qui rendrait service au renderer, qui
aura le meme besoin.

### 4. `importScripts` ou modules

Le plus simple est de n'accepter que les modules ES, deja implementes. Les
pages reelles emploient encore beaucoup `importScripts()`, qui est synchrone —
donc simple a implementer et penible a tester.

## Ce qu'il ne faudrait **pas** faire tout de suite

* **`SharedArrayBuffer`.** Il demande une memoire reellement partagee entre
  deux runtimes QuickJS, et `Atomics` par-dessus. Le noyau saurait le porter —
  `memfd` + `futex` cle par adresse physique —, mais QuickJS non, et ce serait
  le chemin le plus court vers une corruption silencieuse.
* **Les `SharedWorker` et les `ServiceWorker`.** Le premier demande un cycle de
  vie qui survit aux documents, le second un intercepteur de requetes et un
  cache persistant. Ni l'un ni l'autre n'apprend quoi que ce soit de plus sur
  l'isolation.
* **Les workers imbriques.** Un worker qui cree un worker est legal et rare ;
  la borne de profondeur des contextes s'y applique deja.

## Estimation honnete

| Piece | Difficulte | Risque |
|---|---|---|
| `JSRuntime` distinct par worker | moyenne | **eleve** — corruption si le pont fuit une reference |
| File de messages inter-fils | faible | faible, le motif existe deja |
| Clonage structure par JSON | faible | faible, si les limites sont dites |
| Decoupage du prelude | **longue** | faible, mais fastidieuse |
| `importScripts` | faible | faible |

Le seul point vraiment risque est le premier, et c'est aussi celui qui
apprendrait le plus : c'est exactement la question que posera le renderer, en
plus petit et sans surface partagee a gerer.

## Recommandation

A faire **avant** le renderer separe, et pas apres. Un worker eprouve
l'isolation de deux mondes JavaScript pour une fraction du cout, sur une
surface ou un defaut se diagnostique — tout est encore dans le meme processus,
donc dans le meme debogueur. Decouvrir les memes problemes a travers une
frontiere de processus coute dix fois plus cher.
