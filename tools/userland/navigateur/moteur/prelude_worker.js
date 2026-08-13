// `WorkerGlobalScope` : le monde d'un Worker, ecrit pour lui.
//
// ## Pourquoi un fichier a part
//
// La tentation etait de reprendre le prelude de la fenetre et d'en retirer
// `document`, `window` et le DOM. C'eut ete une erreur, et une erreur durable :
//
//   * on ne sait jamais si le retrait est complet. Trois mille lignes ecrites
//     en supposant qu'un document existe s'appuient dessus a des endroits qu'on
//     ne trouve qu'a l'usage — un accesseur qui lit `document.body`, un
//     evenement qui remonte a `window` ;
//   * ce qu'un Worker **a** cesserait d'etre lisible. Un lecteur devrait
//     soustraire mentalement pour savoir ce qui existe ;
//   * chaque ajout au prelude de la fenetre arriverait par accident dans le
//     Worker, ou n'y arriverait pas, sans que personne ne l'ait decide.
//
// Ce fichier dit donc positivement ce qu'un Worker possede. Il est court, et
// c'est voulu : ce qui manque se voit, et s'ajoute a dessein.
//
// ## Ce qu'il n'y a pas, et qui ne doit pas y etre
//
// `document`, `window`, `HTMLElement`, `localStorage`, `alert`. Un Worker n'a
// pas de DOM : c'est ce qui permet de le faire tourner sur un autre fil sans
// verrouiller quoi que ce soit. Lui donner un DOM reviendrait a partager
// l'arbre entre deux fils, c'est-a-dire a rendre le moteur non deterministe.

(function () {
    "use strict";

    const appel = globalThis.__bo_appel;

    // --- Portee globale ------------------------------------------------------
    //
    // Dans un Worker, `self` est l'objet global. Ce n'est pas un alias de
    // `window` : `window` n'existe pas, et du code qui teste `typeof window`
    // pour savoir ou il tourne doit obtenir `"undefined"`.
    globalThis.self = globalThis;

    function DedicatedWorkerGlobalScope() {}
    globalThis.DedicatedWorkerGlobalScope = DedicatedWorkerGlobalScope;
    globalThis.WorkerGlobalScope = DedicatedWorkerGlobalScope;

    // --- Evenements ----------------------------------------------------------
    //
    // Une implementation minuscule et complete, plutot qu'un emprunt au DOM :
    // un Worker n'a que quelques types d'evenements, et ils ne se propagent
    // nulle part — il n'y a pas d'arbre.

    class Event {
        constructor(type, init) {
            this.type = String(type);
            this.defaultPrevented = false;
            this.cancelable = !!(init && init.cancelable);
            this.target = globalThis;
            this.currentTarget = globalThis;
            this.timeStamp = Date.now();
        }
        preventDefault() { if (this.cancelable) this.defaultPrevented = true; }
        stopPropagation() {}
        stopImmediatePropagation() {}
    }
    globalThis.Event = Event;

    class MessageEvent extends Event {
        constructor(type, init) {
            super(type, init);
            this.data = init && "data" in init ? init.data : null;
            this.origin = (init && init.origin) || "";
            this.source = (init && init.source) || null;
            this.lastEventId = "";
            this.ports = (init && init.ports) || [];
        }
    }
    globalThis.MessageEvent = MessageEvent;

    class ErrorEvent extends Event {
        constructor(type, init) {
            super(type, init);
            this.message = (init && init.message) || "";
            this.filename = (init && init.filename) || "";
            this.lineno = (init && init.lineno) || 0;
            this.error = (init && init.error) || null;
        }
    }
    globalThis.ErrorEvent = ErrorEvent;

    // Les ecouteurs vivent sur leur cible, dans `__ecouteurs`. Pas de registre
    // central : `WebSocket` et `IDBRequest` — qui viennent du fichier partage
    // avec la fenetre — sont des cibles comme `globalThis`, et le meme couple
    // attache/emet doit valoir pour toutes.

    function attacheSur(cible, type, f, options) {
        if (typeof f !== "function" && !(f && typeof f.handleEvent === "function"))
            return;
        if (!cible.__ecouteurs) cible.__ecouteurs = new Map();
        const cle = String(type);
        if (!cible.__ecouteurs.has(cle)) cible.__ecouteurs.set(cle, []);
        cible.__ecouteurs.get(cle).push({ f: f, once: !!(options && options.once) });
    }

    function detacheSur(cible, type, f) {
        const liste = cible.__ecouteurs && cible.__ecouteurs.get(String(type));
        if (!liste) return;
        const rang = liste.findIndex((e) => e.f === f);
        if (rang >= 0) liste.splice(rang, 1);
    }

    function emetSur(cible, evenement) {
        const registre = cible.__ecouteurs;
        const liste = registre ? (registre.get(evenement.type) || []).slice() : [];
        for (const entree of liste) {
            if (entree.once) detacheSur(cible, evenement.type, entree.f);
            try {
                if (typeof entree.f === "function") entree.f.call(cible, evenement);
                else entree.f.handleEvent(evenement);
            } catch (e) { signaleErreur(e); }
        }
        const propre = "on" + evenement.type;
        if (typeof cible[propre] === "function") {
            try { cible[propre].call(cible, evenement); }
            catch (e) { signaleErreur(e); }
        }
    }

    globalThis.__ecouteurs = new Map();

    globalThis.addEventListener = function (type, f, options) {
        attacheSur(globalThis, type, f, options);
    };
    globalThis.removeEventListener = function (type, f) {
        detacheSur(globalThis, type, f);
    };
    globalThis.dispatchEvent = function (evenement) {
        emet(evenement);
        return !evenement.defaultPrevented;
    };

    function emet(evenement) {
        emetSur(globalThis, evenement);
    }

    function signaleErreur(e) {
        // Une erreur dans un Worker ne doit pas disparaitre : elle remonte au
        // document qui l'a cree, sans quoi un worker casse est indiscernable
        // d'un worker lent.
        try {
            appel("wkErreur", String((e && e.message) || e),
                  String((e && e.stack) || ""));
        } catch (_) { /* le pont est parti : plus rien a signaler */ }
    }
    globalThis.__bo_signale_erreur = signaleErreur;

    // --- Messages ------------------------------------------------------------

    globalThis.postMessage = function (donnees, transfert) {
        // Le meme clonage que partout ailleurs, et pour la meme raison : ce qui
        // traverse est une copie, jamais une reference. Deux fils qui
        // partageraient un objet partageraient un tas QuickJS, c'est-a-dire
        // exactement ce qu'un Worker existe pour eviter.
        appel("wkVersHote", globalThis.__bo_clone.serialise(donnees));
        void transfert;
    };

    // Appele par l'hote au battement du worker. Jamais au milieu d'un script :
    // c'est la meme discipline que la boucle d'evenements du document.
    globalThis.__bo_worker_message = function (paquet) {
        let donnees;
        try {
            donnees = globalThis.__bo_clone.materialise(paquet);
        } catch (e) {
            emet(new MessageEvent("messageerror", { data: null }));
            return;
        }
        emet(new MessageEvent("message", { data: donnees }));
    };

    globalThis.close = function () { appel("wkFerme"); };

    // --- Minuteries ----------------------------------------------------------
    //
    // Les memes qu'un document, et par le meme chemin : c'est l'hote qui tient
    // les echeances, le worker ne fait qu'executer ce qu'on lui rend.

    const rappels = new Map();
    let prochainRappel = 1;

    function pose(f, delai, repete, arguments_) {
        if (typeof f !== "function") {
            const source = String(f);
            f = function () { (0, eval)(source); };
        }
        const identifiant = prochainRappel++;
        rappels.set(identifiant, { f: f, args: arguments_ || [], repete: repete });
        appel("wkMinuterie", identifiant, Number(delai) || 0, !!repete);
        return identifiant;
    }

    globalThis.setTimeout = function (f, delai) {
        return pose(f, delai, false, Array.prototype.slice.call(arguments, 2));
    };
    globalThis.setInterval = function (f, delai) {
        return pose(f, delai, true, Array.prototype.slice.call(arguments, 2));
    };
    globalThis.clearTimeout = function (identifiant) {
        rappels.delete(identifiant);
        appel("wkAnnuleMinuterie", identifiant);
    };
    globalThis.clearInterval = globalThis.clearTimeout;

    globalThis.__bo_worker_minuterie = function (identifiant) {
        const entree = rappels.get(identifiant);
        if (!entree) return false;
        if (!entree.repete) rappels.delete(identifiant);
        try { entree.f.apply(globalThis, entree.args); }
        catch (e) { signaleErreur(e); }
        return entree.repete;
    };

    globalThis.queueMicrotask = function (f) {
        if (typeof f !== "function") throw new TypeError("fonction attendue");
        // Une vraie microtache : elle s'execute avant le prochain battement,
        // pas au suivant. `Promise.resolve().then` est le seul moyen d'y
        // arriver sans toucher au moteur.
        Promise.resolve().then(function () {
            try { f(); } catch (e) { signaleErreur(e); }
        });
    };

    // --- console -------------------------------------------------------------

    function journalise(niveau) {
        return function () {
            const morceaux = [];
            for (const argument of arguments) {
                if (typeof argument === "string") morceaux.push(argument);
                else {
                    try { morceaux.push(JSON.stringify(argument)); }
                    catch (_) { morceaux.push(String(argument)); }
                }
            }
            appel("wkConsole", niveau, morceaux.join(" "));
        };
    }
    globalThis.console = {
        log: journalise("log"), info: journalise("info"),
        warn: journalise("warn"), error: journalise("error"),
        debug: journalise("debug"), trace: journalise("log"),
    };

    // --- Reseau --------------------------------------------------------------
    //
    // **La meme infrastructure que la fenetre.** L'operation `requete` est
    // celle du document : meme pool de fils, meme CORS, meme politique de
    // schema. Dupliquer Fetch ici aurait donne deux comportements a maintenir,
    // et l'un des deux aurait fini par diverger sur un detail de securite.

    const requetes = new Map();
    let prochaineRequete = 1;

    class Headers {
        constructor(init) {
            this.__map = new Map();
            if (init) for (const cle of Object.keys(init))
                this.__map.set(String(cle).toLowerCase(), String(init[cle]));
        }
        get(nom) { const v = this.__map.get(String(nom).toLowerCase()); return v === undefined ? null : v; }
        has(nom) { return this.__map.has(String(nom).toLowerCase()); }
        set(nom, valeur) { this.__map.set(String(nom).toLowerCase(), String(valeur)); }
        forEach(f) { this.__map.forEach((v, k) => f(v, k, this)); }
        entries() { return this.__map.entries(); }
        keys() { return this.__map.keys(); }
        values() { return this.__map.values(); }
    }
    globalThis.Headers = Headers;

    class Response {
        constructor(reponse) {
            this.status = reponse.status || 0;
            this.statusText = reponse.statusText || "";
            this.ok = this.status >= 200 && this.status < 300;
            this.url = reponse.url || "";
            this.redirected = false;
            this.type = reponse.type || "basic";
            this.headers = new Headers(reponse.headers || {});
            this.__texte = reponse.text || "";
            this.bodyUsed = false;
        }
        text() { this.bodyUsed = true; return Promise.resolve(this.__texte); }
        json() {
            this.bodyUsed = true;
            try { return Promise.resolve(JSON.parse(this.__texte)); }
            catch (e) { return Promise.reject(new SyntaxError(String(e))); }
        }
        clone() { return this; }
    }
    globalThis.Response = Response;

    globalThis.fetch = function (url, options) {
        options = options || {};
        return new Promise(function (resoudre, rejeter) {
            const identifiant = prochaineRequete++;
            requetes.set(identifiant, function (reponse) {
                requetes.delete(identifiant);
                if (!reponse || !reponse.status) {
                    rejeter(new TypeError("echec du chargement : " + url));
                    return;
                }
                resoudre(new Response(reponse));
            });
            let corps = options.body;
            if (corps === undefined || corps === null) corps = null;
            else if (typeof corps !== "string") corps = String(corps);
            appel("wkRequete", identifiant,
                  String(options.method || "GET").toUpperCase(), String(url),
                  corps, options.headers || {},
                  String(options.credentials || "same-origin"));
        });
    };

    globalThis.__bo_worker_reponse = function (identifiant, reponse) {
        const rappel = requetes.get(identifiant);
        if (rappel) rappel(reponse);
    };

    // --- URL -----------------------------------------------------------------

    class URLSearchParams {
        constructor(init) {
            this.__paires = [];
            if (typeof init === "string") {
                for (const morceau of init.replace(/^\?/, "").split("&")) {
                    if (!morceau) continue;
                    const egal = morceau.indexOf("=");
                    const cle = egal < 0 ? morceau : morceau.slice(0, egal);
                    const valeur = egal < 0 ? "" : morceau.slice(egal + 1);
                    this.__paires.push([decodeURIComponent(cle.replace(/\+/g, " ")),
                                        decodeURIComponent(valeur.replace(/\+/g, " "))]);
                }
            } else if (init && typeof init === "object") {
                for (const cle of Object.keys(init))
                    this.__paires.push([String(cle), String(init[cle])]);
            }
        }
        get(nom) {
            const p = this.__paires.find((x) => x[0] === String(nom));
            return p ? p[1] : null;
        }
        getAll(nom) {
            return this.__paires.filter((x) => x[0] === String(nom)).map((x) => x[1]);
        }
        has(nom) { return this.__paires.some((x) => x[0] === String(nom)); }
        set(nom, valeur) {
            const rang = this.__paires.findIndex((x) => x[0] === String(nom));
            if (rang >= 0) {
                this.__paires[rang][1] = String(valeur);
                this.__paires = this.__paires.filter(
                    (x, i) => i <= rang || x[0] !== String(nom));
            } else this.__paires.push([String(nom), String(valeur)]);
        }
        append(nom, valeur) { this.__paires.push([String(nom), String(valeur)]); }
        delete(nom) {
            this.__paires = this.__paires.filter((x) => x[0] !== String(nom));
        }
        forEach(f) { for (const [k, v] of this.__paires) f(v, k, this); }
        entries() { return this.__paires[Symbol.iterator](); }
        keys() { return this.__paires.map((x) => x[0])[Symbol.iterator](); }
        values() { return this.__paires.map((x) => x[1])[Symbol.iterator](); }
        toString() {
            return this.__paires.map(
                (x) => encodeURIComponent(x[0]) + "=" + encodeURIComponent(x[1])
            ).join("&");
        }
    }
    globalThis.URLSearchParams = URLSearchParams;

    class URL {
        constructor(adresse, base) {
            // La resolution passe par l'hote : c'est le meme `urljoin` que le
            // reste du moteur, donc les memes regles pour les adresses
            // relatives, les schemas et les ports par defaut.
            const morceaux = appel("wkAnalyseUrl", String(adresse),
                                   base === undefined ? null : String(base));
            if (!morceaux) throw new TypeError("URL invalide : " + adresse);
            this.href = morceaux.href;
            this.protocol = morceaux.protocol;
            this.hostname = morceaux.hostname;
            this.port = morceaux.port;
            this.host = morceaux.host;
            this.origin = morceaux.origin;
            this.pathname = morceaux.pathname;
            this.search = morceaux.search;
            this.hash = morceaux.hash;
            this.searchParams = new URLSearchParams(this.search);
        }
        toString() { return this.href; }
        toJSON() { return this.href; }
    }
    globalThis.URL = URL;

    // --- Texte ---------------------------------------------------------------
    //
    // UTF-8 seulement. C'est le seul encodage que la norme impose a
    // `TextEncoder`, et le seul que `TextDecoder` doit accepter par defaut.

    class TextEncoder {
        get encoding() { return "utf-8"; }
        encode(texte) {
            texte = texte === undefined ? "" : String(texte);
            const octets = [];
            for (let i = 0; i < texte.length; i++) {
                let point = texte.codePointAt(i);
                if (point > 0xFFFF) i++;
                if (point < 0x80) octets.push(point);
                else if (point < 0x800) {
                    octets.push(0xC0 | (point >> 6), 0x80 | (point & 0x3F));
                } else if (point < 0x10000) {
                    octets.push(0xE0 | (point >> 12),
                                0x80 | ((point >> 6) & 0x3F),
                                0x80 | (point & 0x3F));
                } else {
                    octets.push(0xF0 | (point >> 18),
                                0x80 | ((point >> 12) & 0x3F),
                                0x80 | ((point >> 6) & 0x3F),
                                0x80 | (point & 0x3F));
                }
            }
            return new Uint8Array(octets);
        }
    }
    globalThis.TextEncoder = TextEncoder;

    class TextDecoder {
        constructor(etiquette) {
            const nom = String(etiquette || "utf-8").toLowerCase();
            if (nom !== "utf-8" && nom !== "utf8" && nom !== "unicode-1-1-utf-8")
                throw new RangeError("encodage non pris en charge : " + etiquette);
        }
        get encoding() { return "utf-8"; }
        decode(donnees) {
            if (donnees === undefined || donnees === null) return "";
            const octets = donnees instanceof Uint8Array
                ? donnees : new Uint8Array(donnees.buffer || donnees);
            let sortie = "";
            for (let i = 0; i < octets.length;) {
                const premier = octets[i];
                let point, longueur;
                if (premier < 0x80) { point = premier; longueur = 1; }
                else if ((premier & 0xE0) === 0xC0) { point = premier & 0x1F; longueur = 2; }
                else if ((premier & 0xF0) === 0xE0) { point = premier & 0x0F; longueur = 3; }
                else if ((premier & 0xF8) === 0xF0) { point = premier & 0x07; longueur = 4; }
                else { sortie += "�"; i++; continue; }
                if (i + longueur > octets.length) { sortie += "�"; break; }
                for (let j = 1; j < longueur; j++)
                    point = (point << 6) | (octets[i + j] & 0x3F);
                sortie += String.fromCodePoint(point);
                i += longueur;
            }
            return sortie;
        }
    }
    globalThis.TextDecoder = TextDecoder;

    // --- Stockage et temps reel ----------------------------------------------
    //
    // Les memes services que la fenetre, par les memes operations. Un Worker
    // qui ouvre une base la partage avec son document — c'est ce que dit la
    // norme, et c'est ce qui rend un worker de synchronisation possible.

    // Le joint vers `prelude_partage.js`. L'hote l'invoque a l'amorce, une fois
    // ce fichier evalue : `WebSocket` et `indexedDB` s'installent alors ici
    // avec les primitives d'evenement ci-dessus, et le code qui les definit est
    // celui-la meme qui sert a la fenetre.
    // Ce qui ne traverse pas, cote Worker. La liste est courte parce qu'un
    // Worker n'a pas de DOM : c'est la meme regle appliquee a un monde plus
    // petit, pas une regle differente.
    globalThis.__bo_clone.refuse(globalThis.WebSocket, "WebSocket");
    globalThis.__bo_clone.refuse(globalThis.IDBRequest, "IDBRequest");
    globalThis.__bo_clone.refuse(globalThis.IDBDatabase, "IDBDatabase");
    globalThis.__bo_clone.refuse(globalThis.IDBTransaction, "IDBTransaction");

    globalThis.__bo_worker_env = {
        appel: appel,
        Event: Event,
        attache: function (cible, type, f, o) { attacheSur(cible, type, f, o); },
        detache: function (cible, type, f) { detacheSur(cible, type, f); },
        emet: function (cible, evenement) { emetSur(cible, evenement); },
    };

    globalThis.performance = {
        now: function () { return appel("wkTemps") || 0; },
        timeOrigin: 0,
    };

    globalThis.location = null;   // pose par l'hote a l'amorce
    globalThis.navigator = {
        userAgent: "Bouchaud/1.0 Worker",
        hardwareConcurrency: 1,
        onLine: true,
    };

    // `importScripts` : synchrone, comme la norme le veut. L'hote rapporte la
    // source, on l'evalue dans la portee globale.
    globalThis.importScripts = function () {
        for (const adresse of arguments) {
            const source = appel("wkImporte", String(adresse));
            if (source === null || source === undefined)
                throw new Error("importScripts : " + adresse + " introuvable");
            (0, eval)(source);
        }
    };
})();
