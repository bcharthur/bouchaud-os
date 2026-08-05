// Ce que le JavaScript d'une page trouve en arrivant.
//
// QuickJS apporte le langage — objets, classes, `Promise`, expressions
// rationnelles, `JSON`. Il n'apporte rien du navigateur : ni `window`, ni
// `document`, ni `setTimeout`, ni `fetch`. Ce fichier construit tout cela,
// au-dessus d'une seule primitive fournie par l'hote :
//
//     __bo_appel(operation, ...arguments)
//
// qui aboutit a `moteur/js.py`. Ecrire le DOM ici plutot qu'en C a une
// consequence pratique : ajouter une methode, c'est ajouter une fonction dans
// ce fichier et une branche dans `js.py`, sans recompiler quoi que ce soit.

(function () {
    "use strict";

    const appel = globalThis.__bo_appel;

    // --- Console ------------------------------------------------------------

    function formate(valeur, profondeur) {
        if (profondeur === undefined) profondeur = 0;
        if (valeur === null) return "null";
        if (valeur === undefined) return "undefined";
        const type = typeof valeur;
        if (type === "string") return profondeur === 0 ? valeur : JSON.stringify(valeur);
        if (type === "number" || type === "boolean" || type === "bigint") return String(valeur);
        if (type === "function") return "[Function " + (valeur.name || "anonyme") + "]";
        if (valeur instanceof Error) return valeur.stack || (valeur.name + ": " + valeur.message);
        if (profondeur > 3) return "…";
        if (Array.isArray(valeur))
            return "[" + valeur.map((v) => formate(v, profondeur + 1)).join(", ") + "]";
        if (valeur instanceof Nœud) return "<" + valeur.tagName.toLowerCase() + ">";
        try {
            const paires = Object.keys(valeur).map(
                (c) => c + ": " + formate(valeur[c], profondeur + 1));
            return "{" + paires.join(", ") + "}";
        } catch (e) {
            return String(valeur);
        }
    }

    function journalise(niveau) {
        return function () {
            const morceaux = Array.prototype.map.call(arguments, (v) => formate(v));
            appel("console", niveau, morceaux.join(" "));
        };
    }

    const console = {
        log: journalise("log"),
        info: journalise("info"),
        warn: journalise("warn"),
        error: journalise("error"),
        debug: journalise("debug"),
        trace: journalise("debug"),
        dir: journalise("log"),
    };
    console.table = console.log;
    console.group = console.log;
    console.groupEnd = function () {};
    console.time = function () {};
    console.timeEnd = function () {};
    console.assert = function (condition) {
        if (!condition) console.error.apply(null, Array.prototype.slice.call(arguments, 1));
    };

    // --- Identite des nœuds -------------------------------------------------
    //
    // Cote Python un nœud est un entier. Ici, chaque entier n'a qu'un seul objet
    // enveloppe, garde dans cette table : c'est ce qui fait que
    // `document.body === document.body` est vrai, et qu'on peut poser des
    // ecouteurs sur un nœud retrouve deux fois par deux chemins differents.

    const enveloppes = new Map();

    function noeud(identifiant) {
        if (identifiant === null || identifiant === undefined) return null;
        let objet = enveloppes.get(identifiant);
        if (!objet) {
            if (appel("type", identifiant) === 3) {
                objet = new Texte(identifiant);
            } else {
                const balise = appel("balise", identifiant);
                objet = (balise === "video" || balise === "audio")
                    ? new ElementMedia(identifiant)
                    : new Element(identifiant);
            }
            enveloppes.set(identifiant, objet);
        }
        return objet;
    }

    function noeuds(identifiants) {
        return (identifiants || []).map(noeud);
    }

    function identifiantDe(valeur) {
        return valeur instanceof Nœud ? valeur.__id : null;
    }

    // --- Nœud, Texte, Element -----------------------------------------------

    class Nœud {
        constructor(identifiant) {
            this.__id = identifiant;
            this.__ecouteurs = null;
        }

        get nodeType() { return appel("type", this.__id); }
        get parentNode() { return noeud(appel("parent", this.__id)); }
        get parentElement() { return noeud(appel("parent", this.__id)); }
        get childNodes() { return noeuds(appel("enfants", this.__id, false)); }
        get firstChild() { return noeuds(appel("enfants", this.__id, false))[0] || null; }
        get lastChild() {
            const liste = noeuds(appel("enfants", this.__id, false));
            return liste[liste.length - 1] || null;
        }
        get nextSibling() { return noeud(appel("frere", this.__id, true, false)); }
        get previousSibling() { return noeud(appel("frere", this.__id, false, false)); }
        get ownerDocument() { return document; }

        get textContent() { return appel("texte", this.__id); }
        set textContent(valeur) { appel("poseTexte", this.__id, String(valeur)); }

        appendChild(enfant) {
            appel("insere", this.__id, identifiantDe(enfant), null);
            return enfant;
        }
        insertBefore(enfant, reference) {
            appel("insere", this.__id, identifiantDe(enfant), identifiantDe(reference));
            return enfant;
        }
        removeChild(enfant) {
            appel("retire", identifiantDe(enfant));
            return enfant;
        }
        replaceChild(neuf, ancien) {
            appel("insere", this.__id, identifiantDe(neuf), identifiantDe(ancien));
            appel("retire", identifiantDe(ancien));
            return ancien;
        }
        remove() { appel("retire", this.__id); }
        contains(autre) {
            let courant = autre;
            while (courant) {
                if (courant === this) return true;
                courant = courant.parentNode;
            }
            return false;
        }
        cloneNode(profond) { return noeud(appel("clone", this.__id, !!profond)); }

        // --- Evenements ---
        addEventListener(type, fonction, options) {
            if (typeof fonction !== "function" &&
                !(fonction && typeof fonction.handleEvent === "function")) return;
            if (!this.__ecouteurs) this.__ecouteurs = new Map();
            const capture = !!(options === true || (options && options.capture));
            const cle = type + (capture ? "capture" : "");
            if (!this.__ecouteurs.has(cle)) this.__ecouteurs.set(cle, []);
            const liste = this.__ecouteurs.get(cle);
            if (liste.indexOf(fonction) < 0) liste.push(fonction);
        }
        removeEventListener(type, fonction, options) {
            if (!this.__ecouteurs) return;
            const capture = !!(options === true || (options && options.capture));
            const liste = this.__ecouteurs.get(type + (capture ? "capture" : ""));
            if (!liste) return;
            const position = liste.indexOf(fonction);
            if (position >= 0) liste.splice(position, 1);
        }
        dispatchEvent(evenement) {
            distribue(this, evenement);
            return !evenement.defaultPrevented;
        }
    }

    class Texte extends Nœud {
        get nodeName() { return "#text"; }
        get data() { return appel("texte", this.__id); }
        set data(valeur) { appel("poseTexte", this.__id, String(valeur)); }
        get nodeValue() { return this.data; }
        set nodeValue(valeur) { this.data = valeur; }
        get length() { return this.data.length; }
    }

    /// `element.style` : lecture et ecriture des proprietes CSS en ligne.
    class Style {
        constructor(identifiant) { this.__id = identifiant; }
        getPropertyValue(nom) { return appel("style", this.__id, nom) || ""; }
        setProperty(nom, valeur) { appel("poseStyle", this.__id, nom, String(valeur)); }
        removeProperty(nom) { appel("poseStyle", this.__id, nom, null); }
        get cssText() { return appel("attribut", this.__id, "style") || ""; }
        set cssText(valeur) { appel("poseAttribut", this.__id, "style", String(valeur)); }
    }

    // `element.style.backgroundColor` doit marcher sans qu'on enumere les
    // centaines de proprietes CSS : le mandataire traduit `backgroundColor` en
    // `background-color` a la volee.
    function styleDe(identifiant) {
        return new Proxy(new Style(identifiant), {
            get(cible, propriete) {
                if (propriete in cible) return cible[propriete];
                if (typeof propriete !== "string") return undefined;
                return cible.getPropertyValue(tiret(propriete));
            },
            set(cible, propriete, valeur) {
                if (propriete in cible) { cible[propriete] = valeur; return true; }
                if (typeof propriete === "string") cible.setProperty(tiret(propriete), valeur);
                return true;
            },
        });
    }

    function tiret(nom) {
        return nom.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase());
    }

    /// `element.classList`.
    class ListeClasses {
        constructor(element) { this.__element = element; }
        __lit() {
            const brut = this.__element.getAttribute("class") || "";
            return brut.split(/\s+/).filter((c) => c.length > 0);
        }
        __ecrit(liste) { this.__element.setAttribute("class", liste.join(" ")); }
        contains(nom) { return this.__lit().indexOf(nom) >= 0; }
        add() {
            const liste = this.__lit();
            for (const nom of arguments) if (liste.indexOf(nom) < 0) liste.push(nom);
            this.__ecrit(liste);
        }
        remove() {
            const exclus = Array.prototype.slice.call(arguments);
            this.__ecrit(this.__lit().filter((c) => exclus.indexOf(c) < 0));
        }
        toggle(nom, force) {
            const present = this.contains(nom);
            const veut = force === undefined ? !present : !!force;
            if (veut) this.add(nom); else this.remove(nom);
            return veut;
        }
        item(index) { return this.__lit()[index] || null; }
        get length() { return this.__lit().length; }
        toString() { return this.__lit().join(" "); }
        forEach(f, contexte) { this.__lit().forEach(f, contexte); }
    }

    class Element extends Nœud {
        get tagName() { return (appel("balise", this.__id) || "").toUpperCase(); }
        get nodeName() { return this.tagName; }
        get localName() { return (appel("balise", this.__id) || "").toLowerCase(); }

        getAttribute(nom) { return appel("attribut", this.__id, String(nom)); }
        setAttribute(nom, valeur) {
            appel("poseAttribut", this.__id, String(nom), String(valeur));
        }
        removeAttribute(nom) { appel("poseAttribut", this.__id, String(nom), null); }
        hasAttribute(nom) { return appel("attribut", this.__id, String(nom)) !== null; }
        get attributes() { return appel("attributs", this.__id); }

        get id() { return this.getAttribute("id") || ""; }
        set id(valeur) { this.setAttribute("id", valeur); }
        get className() { return this.getAttribute("class") || ""; }
        set className(valeur) { this.setAttribute("class", valeur); }
        get classList() { return new ListeClasses(this); }

        get innerHTML() { return appel("html", this.__id); }
        set innerHTML(valeur) { appel("poseHtml", this.__id, String(valeur)); }
        get outerHTML() { return appel("html", this.__id, true); }
        get innerText() { return appel("texte", this.__id); }
        set innerText(valeur) { this.textContent = valeur; }

        get children() { return noeuds(appel("enfants", this.__id, true)); }
        get childElementCount() { return this.children.length; }
        get firstElementChild() { return this.children[0] || null; }
        get lastElementChild() {
            const liste = this.children;
            return liste[liste.length - 1] || null;
        }
        get nextElementSibling() { return noeud(appel("frere", this.__id, true, true)); }
        get previousElementSibling() { return noeud(appel("frere", this.__id, false, true)); }

        querySelector(selecteur) {
            return noeud(appel("select", this.__id, String(selecteur), false));
        }
        querySelectorAll(selecteur) {
            return noeuds(appel("select", this.__id, String(selecteur), true));
        }
        getElementsByTagName(nom) {
            return noeuds(appel("parBalise", this.__id, String(nom)));
        }
        getElementsByClassName(nom) {
            return noeuds(appel("parClasse", this.__id, String(nom)));
        }
        closest(selecteur) {
            let courant = this;
            while (courant && courant instanceof Element) {
                if (courant.matches(selecteur)) return courant;
                courant = courant.parentNode;
            }
            return null;
        }
        matches(selecteur) {
            return appel("correspond", this.__id, String(selecteur));
        }

        get style() { return styleDe(this.__id); }

        getBoundingClientRect() {
            const r = appel("rect", this.__id) || { x: 0, y: 0, width: 0, height: 0 };
            return {
                x: r.x, y: r.y, width: r.width, height: r.height,
                left: r.x, top: r.y, right: r.x + r.width, bottom: r.y + r.height,
            };
        }
        get offsetWidth() { return this.getBoundingClientRect().width; }
        get offsetHeight() { return this.getBoundingClientRect().height; }
        get clientWidth() { return this.getBoundingClientRect().width; }
        get clientHeight() { return this.getBoundingClientRect().height; }

        // Champs de formulaire : `value` est ce que le JavaScript lit et ecrit
        // le plus souvent apres le texte.
        get value() { return this.getAttribute("value") || ""; }
        set value(v) { this.setAttribute("value", v); }
        get checked() { return this.hasAttribute("checked"); }
        set checked(v) { if (v) this.setAttribute("checked", ""); else this.removeAttribute("checked"); }
        get disabled() { return this.hasAttribute("disabled"); }
        set disabled(v) { if (v) this.setAttribute("disabled", ""); else this.removeAttribute("disabled"); }
        get href() { return this.getAttribute("href") || ""; }
        set href(v) { this.setAttribute("href", v); }
        get src() { return this.getAttribute("src") || ""; }
        set src(v) { this.setAttribute("src", v); }

        insertAdjacentHTML(position, texte) {
            appel("insereHtml", this.__id, String(position), String(texte));
        }
        append() {
            for (const element of arguments)
                this.appendChild(typeof element === "string"
                    ? document.createTextNode(element) : element);
        }
        prepend() {
            const premier = this.firstChild;
            for (const element of arguments)
                this.insertBefore(typeof element === "string"
                    ? document.createTextNode(element) : element, premier);
        }
        focus() {}
        blur() {}
        scrollIntoView() {}
        click() { distribue(this, new Event("click", { bubbles: true, cancelable: true })); }
    }

    // --- Elements media -----------------------------------------------------
    //
    // `<video>` et `<audio>` ne different que par ce qu'ils montrent : meme
    // etat, memes commandes, memes evenements. Une seule classe les couvre donc,
    // et Python decide s'il y a une image a peindre.

    class ElementMedia extends Element {
        play() {
            appel("mediaJoue", this.__id);
            declencheMedia(this, "play");
            return Promise.resolve();
        }
        pause() {
            appel("mediaPause", this.__id);
            declencheMedia(this, "pause");
        }
        load() { appel("mediaCharge", this.__id); }
        canPlayType(type) { return appel("mediaSaitLire", String(type)) || ""; }

        get currentTime() { return appel("mediaPosition", this.__id) || 0; }
        set currentTime(valeur) {
            appel("mediaCherche", this.__id, Number(valeur) || 0);
            declencheMedia(this, "seeked");
            declencheMedia(this, "timeupdate");
        }
        get duration() {
            const d = appel("mediaDuree", this.__id);
            return d > 0 ? d : NaN;
        }
        get paused() { return !appel("mediaEnLecture", this.__id); }
        get ended() { return !!appel("mediaTermine", this.__id); }
        get seeking() { return false; }

        get volume() {
            const v = appel("mediaVolume", this.__id, null);
            return v === null ? 1 : v;
        }
        set volume(valeur) {
            appel("mediaVolume", this.__id, Math.max(0, Math.min(1, Number(valeur))));
            declencheMedia(this, "volumechange");
        }
        get muted() { return !!appel("mediaMuet", this.__id, null); }
        set muted(valeur) {
            appel("mediaMuet", this.__id, !!valeur);
            declencheMedia(this, "volumechange");
        }
        get loop() { return this.hasAttribute("loop"); }
        set loop(valeur) {
            if (valeur) this.setAttribute("loop", ""); else this.removeAttribute("loop");
            appel("mediaBoucle", this.__id, !!valeur);
        }

        get videoWidth() { return appel("mediaTaille", this.__id)[0] || 0; }
        get videoHeight() { return appel("mediaTaille", this.__id)[1] || 0; }

        // `readyState` et `networkState` : les valeurs de la norme, ramenees a
        // ce que le lecteur sait reellement dire.
        get readyState() { return appel("mediaPret", this.__id) ? 4 : 0; }
        get networkState() { return appel("mediaPret", this.__id) ? 1 : 0; }
        get buffered() {
            const fin = appel("mediaTampon", this.__id) || 0;
            return {
                length: fin > 0 ? 1 : 0,
                start() { return 0; },
                end() { return fin; },
            };
        }
        get played() { return this.buffered; }
        get seekable() { return this.buffered; }
        get error() { return null; }
        get currentSrc() { return this.getAttribute("src") || ""; }

        get src() { return this.getAttribute("src") || ""; }
        set src(valeur) {
            this.setAttribute("src", String(valeur));
            appel("mediaCharge", this.__id);
        }
        get srcObject() { return this.__srcObject || null; }
        set srcObject(valeur) {
            this.__srcObject = valeur;
            if (valeur && valeur.__urlObjet) {
                this.setAttribute("src", valeur.__urlObjet);
                appel("mediaCharge", this.__id);
            }
        }
    }
    globalThis.HTMLMediaElement = ElementMedia;
    globalThis.HTMLVideoElement = ElementMedia;
    globalThis.HTMLAudioElement = ElementMedia;

    globalThis.Audio = function (source) {
        const element = document.createElement("audio");
        if (source) element.src = source;
        return element;
    };

    function declencheMedia(cible, type) {
        distribue(cible, new Event(type, { bubbles: false, cancelable: false }));
    }

    // Appele par Python quand l'etat d'un lecteur change : c'est ce qui donne
    // `timeupdate`, `ended`, `canplay`… sans que le JavaScript ait a sonder.
    globalThis.__bo_media = function (identifiant, type) {
        const cible = noeud(identifiant);
        if (cible) declencheMedia(cible, type);
    };

    // --- Media Source Extensions --------------------------------------------
    //
    // C'est l'API par laquelle un site de lecture alimente son lecteur : il
    // recupere les segments lui-meme, en HTTP, et les pousse dans un
    // `SourceBuffer`. Le lecteur ne connait donc jamais d'URL de media.

    class SourceBuffer {
        constructor(source, type) {
            this.__source = source;
            this.mode = "segments";
            this.updating = false;
            this.timestampOffset = 0;
            this.appendWindowStart = 0;
            this.appendWindowEnd = Infinity;
            this.__type = type;
            this.__ecouteurs = null;
        }
        appendBuffer(donnees) {
            this.updating = true;
            const octets = appel("mseAjoute", this.__source.__id,
                                 versTableau(donnees));
            this.updating = false;
            // `updateend` doit partir apres le retour de `appendBuffer` : les
            // lecteurs enchainent souvent un `appendBuffer` dans ce gestionnaire,
            // et le declencher tout de suite les ferait recursers.
            const buffer = this;
            Promise.resolve().then(function () {
                emet(buffer, "update");
                emet(buffer, "updateend");
            });
            return octets;
        }
        abort() { this.updating = false; }
        remove() { }
        get buffered() {
            const fin = appel("mseTampon", this.__source.__id) || 0;
            return { length: fin > 0 ? 1 : 0, start() { return 0; }, end() { return fin; } };
        }
        addEventListener(type, f) { Nœud.prototype.addEventListener.call(this, type, f); }
        removeEventListener(type, f) { Nœud.prototype.removeEventListener.call(this, type, f); }
    }

    function emet(cible, type) {
        const evenement = new Event(type, {});
        evenement.target = cible;
        for (const f of ecouteursDe(cible, type, false)) invoque(f, cible, evenement);
        const enLigne = cible["on" + type];
        if (typeof enLigne === "function") invoque(enLigne, cible, evenement);
    }

    function versTableau(donnees) {
        // `appendBuffer` recoit un `ArrayBuffer` ou une vue dessus ; le pont ne
        // fait traverser que des tableaux de nombres.
        if (donnees instanceof ArrayBuffer) return Array.from(new Uint8Array(donnees));
        if (ArrayBuffer.isView(donnees))
            return Array.from(new Uint8Array(donnees.buffer, donnees.byteOffset,
                                             donnees.byteLength));
        if (Array.isArray(donnees)) return donnees;
        return [];
    }

    let prochaineSource = 1;

    class MediaSource {
        constructor() {
            this.__id = prochaineSource++;
            this.__urlObjet = "bo-media:" + this.__id;
            this.readyState = "closed";
            this.duration = NaN;
            this.sourceBuffers = [];
            this.activeSourceBuffers = [];
            this.__ecouteurs = null;
            appel("mseCree", this.__id);
            // `sourceopen` part au tour suivant : le code appelant vient tout
            // juste de construire l'objet et n'a pas encore pose son ecouteur.
            const source = this;
            Promise.resolve().then(function () {
                source.readyState = "open";
                emet(source, "sourceopen");
            });
        }
        addSourceBuffer(type) {
            const tampon = new SourceBuffer(this, type);
            this.sourceBuffers.push(tampon);
            this.activeSourceBuffers.push(tampon);
            appel("mseType", this.__id, String(type));
            return tampon;
        }
        removeSourceBuffer() { }
        endOfStream() {
            this.readyState = "ended";
            appel("mseFin", this.__id);
            emet(this, "sourceended");
        }
        setLiveSeekableRange() { }
        clearLiveSeekableRange() { }
        addEventListener(type, f) { Nœud.prototype.addEventListener.call(this, type, f); }
        removeEventListener(type, f) { Nœud.prototype.removeEventListener.call(this, type, f); }
        static isTypeSupported(type) {
            return appel("mediaSaitLire", String(type)) !== "";
        }
    }
    globalThis.MediaSource = MediaSource;
    globalThis.SourceBuffer = SourceBuffer;

    globalThis.URL = globalThis.URL || {};
    globalThis.URL.createObjectURL = function (objet) {
        if (objet && objet.__urlObjet) return objet.__urlObjet;
        return "bo-blob:" + (prochaineSource++);
    };
    globalThis.URL.revokeObjectURL = function () {};

    globalThis.Node = Nœud;
    globalThis.Element = Element;
    globalThis.HTMLElement = Element;
    globalThis.Text = Texte;

    // --- Evenements ---------------------------------------------------------

    class Event {
        constructor(type, options) {
            options = options || {};
            this.type = String(type);
            this.bubbles = !!options.bubbles;
            this.cancelable = !!options.cancelable;
            this.detail = options.detail;
            this.defaultPrevented = false;
            this.target = null;
            this.currentTarget = null;
            this.__arrete = false;
            this.timeStamp = Date.now();
        }
        preventDefault() { if (this.cancelable) this.defaultPrevented = true; }
        stopPropagation() { this.__arrete = true; }
        stopImmediatePropagation() { this.__arrete = true; this.__arreteTout = true; }
    }
    class CustomEvent extends Event {}
    globalThis.Event = Event;
    globalThis.CustomEvent = CustomEvent;

    function ecouteursDe(cible, type, capture) {
        if (!cible.__ecouteurs) return [];
        const liste = cible.__ecouteurs.get(type + (capture ? "capture" : ""));
        return liste ? liste.slice() : [];
    }

    function invoque(fonction, cible, evenement) {
        try {
            if (typeof fonction === "function") fonction.call(cible, evenement);
            else fonction.handleEvent(evenement);
        } catch (e) {
            console.error("erreur dans un ecouteur " + evenement.type + " :", e);
        }
    }

    /// Distribution avec les trois phases : capture, cible, remontee.
    function distribue(cible, evenement) {
        evenement.target = cible;

        const chemin = [];
        for (let courant = cible; courant; courant = courant.parentNode) chemin.push(courant);

        for (let i = chemin.length - 1; i > 0 && !evenement.__arrete; --i) {
            evenement.currentTarget = chemin[i];
            for (const f of ecouteursDe(chemin[i], evenement.type, true))
                invoque(f, chemin[i], evenement);
        }
        if (!evenement.__arrete) {
            evenement.currentTarget = cible;
            for (const f of ecouteursDe(cible, evenement.type, true))
                invoque(f, cible, evenement);
            for (const f of ecouteursDe(cible, evenement.type, false))
                invoque(f, cible, evenement);
            const enLigne = cible instanceof Element
                ? cible.getAttribute("on" + evenement.type) : null;
            if (enLigne) {
                try {
                    // Un attribut `onclick="..."` est du code, pas une valeur.
                    (new Function("event", enLigne)).call(cible, evenement);
                } catch (e) {
                    console.error("erreur dans on" + evenement.type + " :", e);
                }
            }
        }
        if (evenement.bubbles) {
            for (let i = 1; i < chemin.length && !evenement.__arrete; ++i) {
                evenement.currentTarget = chemin[i];
                for (const f of ecouteursDe(chemin[i], evenement.type, false))
                    invoque(f, chemin[i], evenement);
            }
        }
        evenement.currentTarget = null;
    }

    // Appele par Python : un clic reel, une touche, un chargement.
    //
    // Un identifiant nul designe l'evenement de document (`DOMContentLoaded`,
    // `load`). Il faut alors servir **les deux** cibles : `document` et
    // `window`. C'est la forme que prend l'immense majorite du JavaScript de
    // page — `document.addEventListener('DOMContentLoaded', …)` d'un cote,
    // `window.onload` de l'autre —, et n'en servir qu'une revient a ne jamais
    // executer la moitie des scripts du web.
    globalThis.__bo_evenement = function (identifiant, type, details) {
        const evenement = new Event(type, { bubbles: true, cancelable: true });
        Object.assign(evenement, details || {});

        if (identifiant === null || identifiant === undefined) {
            evenement.target = document;
            for (const cible of [document, globalThis]) {
                evenement.currentTarget = cible;
                for (const f of ecouteursDe(cible, type, true)) invoque(f, cible, evenement);
                for (const f of ecouteursDe(cible, type, false)) invoque(f, cible, evenement);
            }
            const enLigne = { load: "onload", DOMContentLoaded: "onreadystatechange" }[type];
            if (enLigne && typeof globalThis[enLigne] === "function")
                invoque(globalThis[enLigne], globalThis, evenement);
            if (type === "DOMContentLoaded" && typeof document.onreadystatechange === "function")
                invoque(document.onreadystatechange, document, evenement);
            return !evenement.defaultPrevented;
        }

        const cible = noeud(identifiant);
        if (!cible) return false;
        distribue(cible, evenement);
        return !evenement.defaultPrevented;
    };

    // --- Minuteries ---------------------------------------------------------

    const minuteries = new Map();
    let prochaineMinuterie = 1;

    function programme(fonction, delai, repete, arguments_) {
        if (typeof fonction === "string") {
            const source = fonction;
            fonction = function () { (new Function(source))(); };
        }
        if (typeof fonction !== "function") return 0;
        const identifiant = prochaineMinuterie++;
        minuteries.set(identifiant, { fonction: fonction, arguments: arguments_, repete: repete });
        appel("minuterie", identifiant, Math.max(0, Number(delai) || 0), repete);
        return identifiant;
    }

    globalThis.setTimeout = function (fonction, delai) {
        return programme(fonction, delai, false, Array.prototype.slice.call(arguments, 2));
    };
    globalThis.setInterval = function (fonction, delai) {
        return programme(fonction, delai, true, Array.prototype.slice.call(arguments, 2));
    };
    globalThis.clearTimeout = function (identifiant) {
        minuteries.delete(identifiant);
        appel("annuleMinuterie", identifiant);
    };
    globalThis.clearInterval = globalThis.clearTimeout;
    globalThis.requestAnimationFrame = function (fonction) {
        return programme(fonction, 16, false, [Date.now()]);
    };
    globalThis.cancelAnimationFrame = globalThis.clearTimeout;
    globalThis.queueMicrotask = function (fonction) {
        Promise.resolve().then(fonction);
    };

    globalThis.__bo_minuterie = function (identifiant) {
        const entree = minuteries.get(identifiant);
        if (!entree) return false;
        if (!entree.repete) minuteries.delete(identifiant);
        try {
            entree.fonction.apply(null, entree.arguments || []);
        } catch (e) {
            console.error("erreur dans une minuterie :", e);
        }
        return entree.repete;
    };

    // --- Requetes -----------------------------------------------------------

    const requetes = new Map();
    let prochaineRequete = 1;

    class XMLHttpRequest {
        constructor() {
            this.readyState = 0;
            this.status = 0;
            this.statusText = "";
            this.responseText = "";
            this.response = "";
            this.responseType = "";
            this.onreadystatechange = null;
            this.onload = null;
            this.onerror = null;
            this.timeout = 0;
            this.__methode = "GET";
            this.__url = "";
            this.__entetes = {};
            this.__reponseEntetes = {};
            this.__synchrone = false;
        }
        open(methode, url, asynchrone) {
            this.__methode = String(methode || "GET").toUpperCase();
            this.__url = String(url);
            this.__synchrone = asynchrone === false;
            this.readyState = 1;
        }
        setRequestHeader(nom, valeur) { this.__entetes[String(nom)] = String(valeur); }
        getAllResponseHeaders() {
            return Object.keys(this.__reponseEntetes)
                .map((c) => c + ": " + this.__reponseEntetes[c]).join("\r\n");
        }
        getResponseHeader(nom) {
            return this.__reponseEntetes[String(nom).toLowerCase()] || null;
        }
        abort() {}
        send(corps) {
            const identifiant = prochaineRequete++;
            requetes.set(identifiant, this);
            appel("requete", identifiant, this.__methode, this.__url,
                  corps === undefined || corps === null ? null : String(corps),
                  this.__entetes, this.__synchrone);
        }
        __termine(reponse) {
            this.status = reponse.status || 0;
            this.statusText = reponse.statusText || "";
            this.responseText = reponse.text || "";
            this.response = this.responseType === "json"
                ? (function (t) { try { return JSON.parse(t); } catch (e) { return null; } })(this.responseText)
                : this.responseText;
            this.__reponseEntetes = reponse.headers || {};
            this.readyState = 4;
            if (typeof this.onreadystatechange === "function")
                invoque(this.onreadystatechange, this, { type: "readystatechange" });
            const nom = this.status > 0 ? "onload" : "onerror";
            if (typeof this[nom] === "function") invoque(this[nom], this, { type: nom.slice(2) });
        }
    }
    globalThis.XMLHttpRequest = XMLHttpRequest;

    globalThis.__bo_reponse = function (identifiant, reponse) {
        const objet = requetes.get(identifiant);
        if (!objet) return;
        requetes.delete(identifiant);
        if (objet instanceof XMLHttpRequest) objet.__termine(reponse);
        else objet(reponse); // resolution d'un `fetch`
    };

    class Reponse {
        constructor(brut) {
            this.status = brut.status || 0;
            this.statusText = brut.statusText || "";
            this.ok = this.status >= 200 && this.status < 300;
            this.url = brut.url || "";
            this.__texte = brut.text || "";
            this.headers = {
                get: (nom) => (brut.headers || {})[String(nom).toLowerCase()] || null,
                has: (nom) => String(nom).toLowerCase() in (brut.headers || {}),
            };
        }
        text() { return Promise.resolve(this.__texte); }
        json() { return Promise.resolve(JSON.parse(this.__texte)); }
    }

    globalThis.fetch = function (url, options) {
        options = options || {};
        return new Promise(function (resoud, rejette) {
            const identifiant = prochaineRequete++;
            requetes.set(identifiant, function (reponse) {
                if (reponse.status > 0) resoud(new Reponse(reponse));
                else rejette(new TypeError("echec du chargement : " + url));
            });
            appel("requete", identifiant,
                  String(options.method || "GET").toUpperCase(), String(url),
                  options.body === undefined || options.body === null
                      ? null : String(options.body),
                  options.headers || {}, false);
        });
    };

    // --- Document et fenetre ------------------------------------------------

    class Document extends Nœud {
        constructor() { super(appel("racine")); this.readyState = "loading"; }
        get documentElement() { return noeud(appel("racine")); }
        get body() { return noeud(appel("corps")); }
        get head() { return noeud(appel("tete")); }
        get title() { return appel("titre"); }
        set title(valeur) { appel("poseTitre", String(valeur)); }
        get URL() { return appel("url"); }
        get location() { return globalThis.location; }
        get cookie() { return ""; }
        set cookie(_) {}

        getElementById(identifiant) { return noeud(appel("parId", String(identifiant))); }
        querySelector(selecteur) {
            return noeud(appel("select", null, String(selecteur), false));
        }
        querySelectorAll(selecteur) {
            return noeuds(appel("select", null, String(selecteur), true));
        }
        getElementsByTagName(nom) { return noeuds(appel("parBalise", null, String(nom))); }
        getElementsByClassName(nom) { return noeuds(appel("parClasse", null, String(nom))); }
        getElementsByName(nom) {
            return this.querySelectorAll("[name=\"" + String(nom).replace(/"/g, "") + "\"]");
        }
        createElement(nom) { return noeud(appel("creeElement", String(nom))); }
        createElementNS(_, nom) { return this.createElement(nom); }
        createTextNode(texte) { return noeud(appel("creeTexte", String(texte))); }
        createDocumentFragment() { return this.createElement("bo-fragment"); }
        createComment() { return this.createTextNode(""); }
        createEvent() { return new Event("custom", {}); }
        write(texte) { appel("ecrit", String(texte)); }
        writeln(texte) { appel("ecrit", String(texte) + "\n"); }
        open() {}
        close() {}
    }

    const document = new Document();
    globalThis.document = document;

    /// `window.location`, en lecture ; naviguer se demande a l'hote.
    const location = {
        get href() { return appel("url"); },
        set href(valeur) { appel("navigue", String(valeur)); },
        assign(valeur) { appel("navigue", String(valeur)); },
        replace(valeur) { appel("navigue", String(valeur)); },
        reload() { appel("navigue", appel("url")); },
        toString() { return appel("url"); },
    };
    for (const partie of ["protocol", "host", "hostname", "port", "pathname",
                          "search", "hash", "origin"]) {
        Object.defineProperty(location, partie, {
            get() { return appel("urlPartie", partie); },
            enumerable: true,
        });
    }
    globalThis.location = location;

    /// Un stockage qui ne survit pas a la page : il n'y a pas de disque
    /// inscriptible sous cet OS, et une page qui l'utilise doit fonctionner
    /// plutot que lever.
    function stockage() {
        const donnees = new Map();
        return {
            getItem: (c) => (donnees.has(String(c)) ? donnees.get(String(c)) : null),
            setItem: (c, v) => { donnees.set(String(c), String(v)); },
            removeItem: (c) => { donnees.delete(String(c)); },
            clear: () => donnees.clear(),
            key: (i) => Array.from(donnees.keys())[i] || null,
            get length() { return donnees.size; },
        };
    }
    globalThis.localStorage = stockage();
    globalThis.sessionStorage = stockage();

    globalThis.navigator = {
        userAgent: "Mozilla/5.0 (Bouchaud OS; x86_64) BoNavigateur/1.0",
        appName: "Netscape",
        platform: "BouchaudOS",
        language: "fr-FR",
        languages: ["fr-FR", "fr", "en"],
        onLine: true,
        cookieEnabled: false,
        userAgentData: undefined,
    };

    const taille = appel("tailleVue") || { width: 1280, height: 720 };
    globalThis.screen = {
        width: taille.width, height: taille.height,
        availWidth: taille.width, availHeight: taille.height,
        colorDepth: 32, pixelDepth: 32,
    };

    globalThis.history = {
        length: 1,
        pushState() {}, replaceState() {},
        back() { appel("historique", -1); },
        forward() { appel("historique", 1); },
        go(n) { appel("historique", Number(n) || 0); },
    };

    globalThis.console = console;
    globalThis.window = globalThis;
    globalThis.self = globalThis;
    globalThis.top = globalThis;
    globalThis.parent = globalThis;
    globalThis.frames = globalThis;
    globalThis.closed = false;

    Object.defineProperty(globalThis, "innerWidth",
        { get: () => (appel("tailleVue") || taille).width, enumerable: true });
    Object.defineProperty(globalThis, "innerHeight",
        { get: () => (appel("tailleVue") || taille).height, enumerable: true });
    globalThis.outerWidth = taille.width;
    globalThis.outerHeight = taille.height;
    globalThis.devicePixelRatio = 1;
    globalThis.scrollX = 0;
    globalThis.scrollY = 0;
    globalThis.pageXOffset = 0;
    globalThis.pageYOffset = 0;

    globalThis.alert = function (message) { appel("console", "alerte", formate(message)); };
    globalThis.confirm = function () { return false; };
    globalThis.prompt = function () { return null; };
    globalThis.scrollTo = function () {};
    globalThis.scrollBy = function () {};
    globalThis.open = function () { return null; };
    globalThis.close = function () {};
    globalThis.focus = function () {};
    globalThis.blur = function () {};
    globalThis.matchMedia = function (requete) {
        return { matches: false, media: String(requete),
                 addListener() {}, removeListener() {},
                 addEventListener() {}, removeEventListener() {} };
    };
    globalThis.getComputedStyle = function (element) { return element.style; };
    globalThis.btoa = function (texte) { return appel("base64", String(texte), true); };
    globalThis.atob = function (texte) { return appel("base64", String(texte), false); };

    // La fenetre recoit les evenements comme un nœud : c'est ce qu'attend
    // `window.addEventListener('load', …)`, la forme la plus courante de tout
    // le JavaScript de page.
    globalThis.__ecouteurs = new Map();
    globalThis.addEventListener = Nœud.prototype.addEventListener.bind(globalThis);
    globalThis.removeEventListener = Nœud.prototype.removeEventListener.bind(globalThis);
    globalThis.dispatchEvent = function (evenement) {
        for (const f of ecouteursDe(globalThis, evenement.type, false))
            invoque(f, globalThis, evenement);
        return !evenement.defaultPrevented;
    };

    // Etat du document, mis a jour par Python quand l'analyse est finie.
    globalThis.__bo_pret = function () {
        document.readyState = "interactive";
        globalThis.__bo_evenement(null, "DOMContentLoaded", {});
        document.readyState = "complete";
        globalThis.__bo_evenement(null, "load", {});
    };
})();
