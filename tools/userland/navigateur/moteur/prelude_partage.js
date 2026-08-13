// Ce qu'une fenetre et un Worker possedent tous les deux : `WebSocket` et
// `indexedDB`.
//
// ## Pourquoi un fichier a part
//
// Ces deux surfaces sont les seules du moteur qui ne demandent rien au DOM :
// elles parlent a un service — une connexion, une base — et rendent des
// evenements. Un Worker en a besoin mot pour mot, et il n'y avait que deux
// facons de les lui donner : recopier trois cents lignes, ou les ecrire une
// fois pour deux mondes. La copie aurait diverge — pas tout de suite, mais sur
// un detail de fermeture de transaction ou de code de fin de connexion, et
// c'est exactement le genre d'ecart qu'on ne trouve qu'en production.
//
// ## Le joint
//
// Tout ce qui differe entre une fenetre et un Worker passe par `env` :
//
//     env.appel(operation, …)      la porte vers Python — celle du contexte hote
//     env.Event                    la classe d'evenement du monde appelant
//     env.attache(cible, type, f, o)
//     env.detache(cible, type, f, o)
//     env.emet(cible, evenement)   ecouteurs enregistres, puis `on<type>`
//
// Les **noms d'operations sont les memes des deux cotes** (`idbOuvre`,
// `ouvreSocket`, …). C'est voulu : le Python d'un document et celui d'un Worker
// implementent le meme protocole, chacun avec ses propres files, et ce fichier
// n'a pas a savoir lequel des deux lui repond.
//
// Ce que le fichier installe sur son `globalThis` : `WebSocket`, `indexedDB`,
// `IDBRequest`, `IDBDatabase`, `IDBTransaction`, `IDBObjectStore`,
// `IDBFactory`, et les deux portes d'entree `__bo_socket` et `__bo_idb` que
// l'hote appelle au battement.

globalThis.__bo_installe_partage = function (env) {
    // --- MessageChannel -----------------------------------------------------
    //
    // Deux bouts d'un meme fil, dans le meme contexte JavaScript. Contrairement
    // aux deux autres messageries — celle d'un Worker, celle d'un contexte de
    // navigation —, celle-ci ne traverse aucune frontiere : les deux ports
    // vivent cote a cote.
    //
    // Ce qui la rend malgre tout non triviale est la **livraison differee**. Un
    // `port1.postMessage(x)` ne doit pas appeler `port2.onmessage` tout de
    // suite : ce serait un appel de fonction deguise, et du code qui compte sur
    // l'ordre de la boucle d'evenements se tromperait. La livraison passe donc
    // par `setTimeout(…, 0)`, c'est-a-dire par la file de taches que l'hote
    // vidange deja. Aucune troisieme messagerie n'est fabriquee ici : celle-ci
    // emprunte celle des minuteries.
    //
    // Et les donnees sont clonees, comme partout ailleurs — meme entre deux
    // ports du meme contexte. Deux pages qui se partageraient un objet par
    // `postMessage` decouvriraient la difference le jour ou l'une d'elles
    // deviendrait un Worker.

    class MessagePort {
        constructor() {
            this.__ecouteurs = new Map();
            this.__jumeau = null;
            this.__demarre = false;
            this.__attente = [];
            this.__ferme = false;
            this.__onmessage = null;
            this.onmessageerror = null;
        }

        // Poser `onmessage` demarre le port. Ce n'est pas une commodite : la
        // norme l'exige, et sans cela le cas le plus courant — `port.onmessage
        // = f` et rien d'autre — ne recevrait jamais rien. Le port resterait
        // parfaitement silencieux, ce qui est le pire des symptomes.
        get onmessage() { return this.__onmessage; }
        set onmessage(f) {
            this.__onmessage = f;
            if (typeof f === "function") this.start();
        }

        addEventListener(type, f, o) {
            env.attache(this, type, f, o);
            // Poser un `onmessage` ou un ecouteur `message` demarre le port :
            // c'est ce que dit la norme, et c'est ce qui evite d'avoir a
            // appeler `start()` dans le cas courant.
            if (String(type) === "message") this.start();
        }
        removeEventListener(type, f, o) { env.detache(this, type, f, o); }

        start() {
            if (this.__demarre || this.__ferme) return;
            this.__demarre = true;
            const attente = this.__attente;
            this.__attente = [];
            for (const paquet of attente) this.__livre(paquet);
        }

        close() {
            this.__ferme = true;
            this.__attente = [];
        }

        postMessage(donnees, transfert) {
            void transfert;
            if (this.__ferme) return;
            const paquet = globalThis.__bo_clone.serialise(donnees);
            const jumeau = this.__jumeau;
            if (!jumeau || jumeau.__ferme) return;
            // Differe, toujours : livrer ici ferait de `postMessage` un appel
            // synchrone, ce qu'il n'est pas.
            setTimeout(function () { jumeau.__recoit(paquet); }, 0);
        }

        __recoit(paquet) {
            if (this.__ferme) return;
            if (!this.__demarre) { this.__attente.push(paquet); return; }
            this.__livre(paquet);
        }

        __livre(paquet) {
            let donnees;
            try {
                donnees = globalThis.__bo_clone.materialise(paquet);
            } catch (e) {
                const rate = new env.Event("messageerror");
                rate.data = null;
                env.emet(this, rate);
                return;
            }
            const evenement = new env.Event("message");
            evenement.data = donnees;
            evenement.origin = "";
            evenement.source = null;
            evenement.ports = [];
            env.emet(this, evenement);
        }
    }

    class MessageChannel {
        constructor() {
            this.port1 = new MessagePort();
            this.port2 = new MessagePort();
            this.port1.__jumeau = this.port2;
            this.port2.__jumeau = this.port1;
        }
    }

    globalThis.MessagePort = MessagePort;
    globalThis.MessageChannel = MessageChannel;

    // Un port n'est pas clonable : il se **transfere**, et le transfert n'est
    // pas implemente. Le refuser nommement vaut mieux que de le laisser passer
    // pour un objet simple, ce qui donnerait a l'autre bout un port sans
    // jumeau — inerte, et sans rien pour l'expliquer.
    globalThis.__bo_clone.refuse(MessagePort, "MessagePort");

    // --- WebSocket ----------------------------------------------------------
    //
    // La premiere connexion que la page garde ouverte. Tout le reste du reseau
    // est un aller-retour ; ici le serveur parle quand il veut, et ce qu'il dit
    // arrive au battement de la boucle d'evenements — jamais au milieu d'un
    // script.

    const sockets = new Map();
    let prochainSocket = 1;

    class WebSocket {
        constructor(url, protocoles) {
            this.url = String(url);
            this.readyState = 0;              // CONNECTING
            this.bufferedAmount = 0;
            this.extensions = "";
            this.protocol = "";
            this.binaryType = "blob";
            this.onopen = null;
            this.onmessage = null;
            this.onerror = null;
            this.onclose = null;
            this.__ecouteurs = new Map();
            this.__id = prochainSocket++;
            sockets.set(this.__id, this);
            const liste = protocoles === undefined ? []
                : (Array.isArray(protocoles) ? protocoles.map(String)
                                             : [String(protocoles)]);
            env.appel("ouvreSocket", this.__id, this.url, liste);
        }

        addEventListener(type, f, o) {
            env.attache(this, type, f, o);
        }
        removeEventListener(type, f, o) {
            env.detache(this, type, f, o);
        }

        send(donnees) {
            if (this.readyState !== 1)
                throw new Error("WebSocket: la connexion n'est pas ouverte");
            env.appel("envoieSocket", this.__id,
                  typeof donnees === "string" ? donnees : Array.from(donnees));
        }

        close(code, raison) {
            if (this.readyState === 2 || this.readyState === 3) return;
            this.readyState = 2;              // CLOSING
            env.appel("fermeSocket", this.__id, code === undefined ? 1000 : code,
                  raison === undefined ? "" : String(raison));
        }

        __recoit(type, charge) {
            let evenement;
            if (type === "message") {
                evenement = new env.Event("message");
                evenement.data = charge;
                evenement.origin = this.url;
                this.readyState = 1;
            } else if (type === "open") {
                this.readyState = 1;
                // Le sous-protocole retenu par le serveur. La page le lit dans
                // `onopen` pour savoir quelle langue parler — le lire plus tard
                // serait trop tard.
                this.protocol = (charge && charge.protocole) || "";
                evenement = new env.Event("open");
            } else if (type === "close") {
                this.readyState = 3;          // CLOSED
                evenement = new env.Event("close");
                evenement.code = (charge && charge.code) || 1006;
                evenement.reason = (charge && charge.raison) || "";
                evenement.wasClean = !!(charge && charge.propre);
                sockets.delete(this.__id);
            } else {
                evenement = new env.Event("error");
                evenement.message = String(charge || "");
            }
            env.emet(this, evenement);
        }
    }

    WebSocket.CONNECTING = 0;
    WebSocket.OPEN = 1;
    WebSocket.CLOSING = 2;
    WebSocket.CLOSED = 3;
    WebSocket.prototype.CONNECTING = 0;
    WebSocket.prototype.OPEN = 1;
    WebSocket.prototype.CLOSING = 2;
    WebSocket.prototype.CLOSED = 3;

    globalThis.WebSocket = WebSocket;

    globalThis.__bo_socket = function (identifiant, type, charge) {
        const connexion = sockets.get(identifiant);
        if (connexion) connexion.__recoit(type, charge);
    };

    // --- IndexedDB ----------------------------------------------------------
    //
    // La memoire d'une application Web. `localStorage` ne range que des
    // chaines, n'a ni transaction ni requete asynchrone, et bloque le fil de
    // l'interface a chaque ecriture ; une application qui garde son etat hors
    // ligne a besoin de l'autre.
    //
    // Tout ici rend **immediatement** une `IDBRequest` vide et rend la main.
    // Le travail se fait sur un fil, et son resultat arrive au battement
    // suivant de la boucle d'evenements — donc jamais au milieu d'un script.
    // C'est la meme discipline que le reseau, et c'est ce qui permet a une page
    // d'ecrire mille enregistrements sans que ses minuteries s'arretent.

    const requetesIdb = new Map();
    let prochaineRequeteIdb = 1;

    // Une valeur rangee voyage sous forme de paquet de clonage, marque pour
    // qu'on le reconnaisse a la relecture. Le marqueur est necessaire : une
    // base ecrite avant ce changement contient des valeurs nues, et les lire
    // comme des paquets echouerait au lieu de simplement les rendre.
    const MARQUE = "__bo_clone";

    function emballe(valeur) {
        const paquet = globalThis.__bo_clone.serialise(valeur);
        paquet[MARQUE] = 1;
        return paquet;
    }

    function deballe(valeur) {
        if (!valeur || typeof valeur !== "object" || !valeur[MARQUE])
            return valeur;
        try { return globalThis.__bo_clone.materialise(valeur); }
        catch (e) { return valeur; }
    }

    function evenementIdb(type, cible) {
        const evenement = new env.Event(type);
        // La norme veut `target` **et** `currentTarget` : du code reel lit
        // indifferemment l'un ou l'autre, et n'en trouver qu'un le fait tomber
        // sur `undefined.result`.
        try {
            Object.defineProperty(evenement, "target", { value: cible });
            Object.defineProperty(evenement, "currentTarget", { value: cible });
        } catch (e) { /* deja defini */ }
        return evenement;
    }

    class IDBRequest {
        constructor() {
            this.result = undefined;
            this.error = null;
            this.readyState = "pending";
            this.onsuccess = null;
            this.onerror = null;
            this.onupgradeneeded = null;
            this.onblocked = null;
            this.transaction = null;
            this.source = null;
            this.__ecouteurs = new Map();
            this.__id = prochaineRequeteIdb++;
            requetesIdb.set(this.__id, this);
        }

        addEventListener(type, f, o) {
            env.attache(this, type, f, o);
        }
        removeEventListener(type, f, o) {
            env.detache(this, type, f, o);
        }

        __emet(type) {
            const evenement = evenementIdb(type, this);
            env.emet(this, evenement);
            return evenement;
        }

        __reussit(valeur) {
            // Cote Python il n'y a qu'un `None`, qui traverse le pont en
            // `null`. IndexedDB, lui, distingue « absent » (`undefined`) de
            // « range a null ». Le marqueur retablit la distinction : une page
            // qui teste `if (r.result === undefined)` pour decider s'il faut
            // creer l'enregistrement doit voir la difference.
            if (valeur && typeof valeur === "object" && valeur.__bo_absent)
                valeur = undefined;
            // Une valeur rangee l'a ete sous forme de paquet de clonage : c'est
            // ce qui permet a une `Date`, a un `Map` ou a un objet cyclique de
            // survivre a un aller-retour dans la base. Les autres resultats —
            // un compte, une liste de cles — n'en sont pas et passent tels
            // quels.
            // `getAll` rend une liste de valeurs rangees : chacune porte son
            // propre paquet. Les listes de cles, elles, n'en portent aucun et
            // traversent sans etre touchees.
            valeur = Array.isArray(valeur) ? valeur.map(deballe) : deballe(valeur);
            this.result = valeur;
            this.readyState = "done";
            this.__emet("success");
        }

        __echoue(erreur) {
            // `error` est un objet, pas une chaine : du code reel lit
            // `request.error.name` pour distinguer une contrainte violee d'un
            // quota depasse.
            const objet = new Error(erreur && erreur.message || "erreur IndexedDB");
            objet.name = (erreur && erreur.nom) || "UnknownError";
            this.error = objet;
            this.readyState = "done";
            this.__emet("error");
        }
    }

    class IDBObjectStore {
        constructor(transaction, nom) {
            this.__transaction = transaction;
            this.name = nom;
            this.keyPath = null;
            this.autoIncrement = false;
            this.indexNames = [];
        }

        get transaction() { return this.__transaction; }

        __operation(verbe, cle, valeur) {
            const requete = new IDBRequest();
            requete.source = this;
            requete.transaction = this.__transaction;
            this.__transaction.__enregistre(requete);
            // La **valeur** est clonee, la **cle** non : une cle doit rester
            // comparable et ordonnable cote Python, ou la base ne saurait plus
            // ni retrouver ni trier. C'est aussi ce que dit la norme, qui
            // n'accepte comme cle que des nombres, des chaines, des dates et
            // des tableaux de ceux-la.
            const emballee = ["put", "add"].indexOf(verbe) >= 0
                ? emballe(valeur) : (valeur === undefined ? null : valeur);
            env.appel("idbOperation", requete.__id, this.__transaction.__jeton,
                  verbe, this.name,
                  cle === undefined ? null : cle, emballee);
            return requete;
        }

        // `put` remplace, `add` refuse une cle deja prise. La distinction n'est
        // pas cosmetique : `add` est ce qui protege d'un doublon quand deux
        // chemins de code enregistrent le meme objet.
        put(valeur, cle) { return this.__operation("put", cle, valeur); }
        add(valeur, cle) { return this.__operation("add", cle, valeur); }
        get(cle) { return this.__operation("get", cle, null); }
        delete(cle) { return this.__operation("delete", cle, null); }
        clear() { return this.__operation("clear", null, null); }
        count() { return this.__operation("count", null, null); }
        getAll() { return this.__operation("getAll", null, null); }
        getAllKeys() { return this.__operation("getAllKeys", null, null); }
    }

    class IDBTransaction {
        constructor(base, magasins, mode) {
            this.db = base;
            this.mode = mode;
            this.objectStoreNames = magasins;
            this.error = null;
            this.oncomplete = null;
            this.onerror = null;
            this.onabort = null;
            this.__ecouteurs = new Map();
            this.__jeton = null;
            this.__enAttente = 0;
            this.__terminee = false;
            this.__echouee = false;
        }

        addEventListener(type, f, o) {
            env.attache(this, type, f, o);
        }
        removeEventListener(type, f, o) {
            env.detache(this, type, f, o);
        }

        objectStore(nom) { return new IDBObjectStore(this, nom); }

        __enregistre(requete) {
            this.__enAttente += 1;
            const transaction = this;
            requete.addEventListener("success", function () {
                transaction.__unDeMoins(false);
            });
            requete.addEventListener("error", function () {
                transaction.__unDeMoins(true);
            });
        }

        // La transaction se valide toute seule quand sa derniere requete est
        // revenue. C'est ce que fait un vrai IndexedDB, et c'est ce qui evite a
        // la page d'avoir a appeler un `commit()` que la norme ne demande pas.
        __unDeMoins(echec) {
            if (echec) this.__echouee = true;
            this.__enAttente -= 1;
            if (this.__enAttente <= 0 && !this.__terminee) this.__conclut();
        }

        __conclut() {
            if (this.__terminee) return;
            this.__terminee = true;
            const valide = !this.__echouee;
            const transaction = this;
            const fin = new IDBRequest();
            fin.addEventListener("success", function () {
                transaction.__emet(valide ? "complete" : "abort");
            });
            fin.addEventListener("error", function () {
                transaction.__emet("abort");
            });
            env.appel("idbTermine", fin.__id, this.__jeton, valide);
        }

        // Une transaction abandonnee ne laisse **rien** derriere elle : ni la
        // moitie de ses ecritures, ni un magasin a moitie vide.
        abort() {
            if (this.__terminee) return;
            this.__echouee = true;
            this.__conclut();
        }

        __emet(type) {
            const evenement = evenementIdb(type, this);
            env.emet(this, evenement);
        }
    }

    class IDBDatabase {
        constructor(nom, version, magasins) {
            this.name = nom;
            this.version = version;
            this.objectStoreNames = magasins || [];
            this.onversionchange = null;
            this.onclose = null;
            this.__ecouteurs = new Map();
            // Ce que `onupgradeneeded` a demande, pas encore ecrit.
            this.__creations = [];
            this.__suppressions = [];
            this.__enMontee = false;
        }

        addEventListener(type, f, o) {
            env.attache(this, type, f, o);
        }
        removeEventListener(type, f, o) {
            env.detache(this, type, f, o);
        }

        createObjectStore(nom, options) {
            if (!this.__enMontee)
                throw new Error("createObjectStore hors de onupgradeneeded");
            nom = String(nom);
            this.__creations.push([nom, options || {}]);
            if (this.objectStoreNames.indexOf(nom) < 0)
                this.objectStoreNames.push(nom);
            // Le magasin est utilisable tout de suite du point de vue de la
            // page : c'est ce que fait un vrai IndexedDB pendant une montee.
            return new IDBObjectStore(null, nom);
        }

        deleteObjectStore(nom) {
            if (!this.__enMontee)
                throw new Error("deleteObjectStore hors de onupgradeneeded");
            nom = String(nom);
            this.__suppressions.push(nom);
            const rang = this.objectStoreNames.indexOf(nom);
            if (rang >= 0) this.objectStoreNames.splice(rang, 1);
        }

        transaction(magasins, mode) {
            const liste = Array.isArray(magasins) ? magasins.map(String)
                                                  : [String(magasins)];
            const t = new IDBTransaction(this, liste,
                                         mode === "readwrite" ? "readwrite"
                                                              : "readonly");
            // Le jeton est attribue cote Python ; l'appel le rend directement,
            // ce qui evite d'avoir a differer les operations qui suivent.
            const ouverture = new IDBRequest();
            t.__jeton = env.appel("idbTransaction", ouverture.__id, this.name, t.mode);
            return t;
        }

        close() { /* rien a fermer : les bases ne tiennent pas de descripteur */ }
    }

    class IDBFactory {
        open(nom, version) {
            const requete = new IDBRequest();
            requete.__base = String(nom);
            requete.__versionDemandee = version === undefined ? null : Number(version);
            requete.__etape = "ouverture";
            env.appel("idbOuvre", requete.__id, requete.__base,
                  requete.__versionDemandee);
            return requete;
        }

        deleteDatabase(nom) {
            const requete = new IDBRequest();
            requete.__etape = "suppression";
            env.appel("idbSupprime", requete.__id, String(nom));
            return requete;
        }

        cmp(a, b) { return a < b ? -1 : (a > b ? 1 : 0); }
    }

    globalThis.IDBRequest = IDBRequest;
    globalThis.IDBDatabase = IDBDatabase;
    globalThis.IDBTransaction = IDBTransaction;
    globalThis.IDBObjectStore = IDBObjectStore;
    globalThis.IDBFactory = IDBFactory;
    globalThis.indexedDB = new IDBFactory();

    // Le retour des fils de travail. Une seule porte d'entree, appelee par
    // `tic()` : rien de ce qui suit ne s'execute au milieu d'un script.
    globalThis.__bo_idb = function (identifiant, resultat, erreur) {
        const requete = requetesIdb.get(identifiant);
        if (!requete) return;
        requetesIdb.delete(identifiant);
        if (erreur) { requete.__echoue(erreur); return; }

        if (requete.__etape === "ouverture") {
            const base = new IDBDatabase(requete.__base, resultat.cible,
                                         resultat.magasins);
            if (resultat.montee) {
                // `onupgradeneeded` d'abord, `onsuccess` seulement une fois la
                // montee ecrite. L'ordre n'est pas negociable : une page qui
                // recevrait `onsuccess` avant que ses magasins existent
                // ouvrirait une transaction sur du vide.
                base.__enMontee = true;
                requete.result = base;
                requete.__ancienneVersion = resultat.version;
                const evenement = evenementIdb("upgradeneeded", requete);
                evenement.oldVersion = resultat.version;
                evenement.newVersion = resultat.cible;
                env.emet(requete, evenement);
                base.__enMontee = false;

                const suite = new IDBRequest();
                suite.__etape = "montee";
                suite.__baseObjet = base;
                suite.__origine = requete;
                suite.addEventListener("success", function () {
                    base.version = suite.result.version;
                    base.objectStoreNames = suite.result.magasins;
                    requete.__reussit(base);
                });
                suite.addEventListener("error", function () {
                    requete.__echoue({ nom: suite.error.name,
                                       message: suite.error.message });
                });
                env.appel("idbMonte", suite.__id, base.name, resultat.cible,
                      base.__creations, base.__suppressions);
                base.__creations = [];
                base.__suppressions = [];
                return;
            }
            requete.__reussit(base);
            return;
        }
        requete.__reussit(resultat);
    };
};
