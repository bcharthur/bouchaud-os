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

    // `console.error("%s\n\n%o", message, objet)` est la forme qu'emploient les
    // cadres applicatifs pour leurs messages d'erreur. Sans substitution, le
    // diagnostic arrivait litteralement sous la forme « %s\n\n%o » suivi des
    // valeurs — illisible au moment precis ou l'on en a le plus besoin.
    const _SUBSTITUTION = /%[sdifoOc%]/g;

    function substitue(gabarit, arguments_) {
        let index = 1;
        const rendu = gabarit.replace(_SUBSTITUTION, (marque) => {
            if (marque === "%%") return "%";
            if (marque === "%c") { index++; return ""; }  // style CSS : ignore
            if (index >= arguments_.length) return marque;
            const valeur = arguments_[index++];
            if (marque === "%s") return String(valeur);
            if (marque === "%d" || marque === "%i") return String(parseInt(valeur, 10));
            if (marque === "%f") return String(parseFloat(valeur));
            return formate(valeur, 1);
        });
        const restants = Array.prototype.slice.call(arguments_, index)
            .map((v) => formate(v));
        return restants.length ? rendu + " " + restants.join(" ") : rendu;
    }

    function journalise(niveau) {
        return function () {
            if (typeof arguments[0] === "string" &&
                    _SUBSTITUTION.test(arguments[0])) {
                _SUBSTITUTION.lastIndex = 0;
                appel("console", niveau, substitue(arguments[0], arguments));
                return;
            }
            _SUBSTITUTION.lastIndex = 0;
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

    // Balise -> classe declaree par `customElements.define`. Consultee des
    // la fabrication d'une enveloppe, d'ou sa place ici.
    const definitions = new Map();

    function noeud(identifiant) {
        if (identifiant === null || identifiant === undefined) return null;
        let objet = enveloppes.get(identifiant);
        if (!objet) {
            if (appel("type", identifiant) === 3) {
                objet = new Texte(identifiant);
            } else {
                const balise = appel("balise", identifiant);
                // Un element dont la balise est un composant declare recoit
                // directement la classe du composant : c'est la que la mise a
                // niveau se fait naturellement, sans avoir a remplacer un objet
                // que du code tient peut-etre deja.
                const definition = definitions.get(balise);
                if (definition) {
                    objet = construitComposant(definition, identifiant);
                } else if (balise === "video" || balise === "audio") {
                    objet = new ElementMedia(identifiant);
                } else if (balise === "canvas") {
                    objet = new ElementToile(identifiant);
                } else {
                    objet = new Element(identifiant);
                }
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

    // Identifiant du nœud en cours de construction. Le constructeur d'un
    // composant appelle `super()` sans argument — c'est la forme imposee par la
    // norme — et il faut bien que l'objet sache quel element il enveloppe.
    let enConstruction = null;

    class Nœud {
        constructor(identifiant) {
            this.__id = identifiant === undefined ? enConstruction : identifiant;
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
        set textContent(valeur) {
            const ancienne = appel("texte", this.__id);
            appel("poseTexte", this.__id, String(valeur));
            signale("characterData", this, { ancienne });
        }

        // Chaque modification de l'arbre se declare : c'est ainsi, et seulement
        // ainsi, que `MutationObserver` peut exister — les methodes du DOM sont
        // les seules portes vers l'arbre, rien ne peut changer sans passer ici.
        appendChild(enfant) {
            appel("insere", this.__id, identifiantDe(enfant), null);
            signale("childList", this, { ajoutes: [enfant] });
            connecte(enfant);
            return enfant;
        }
        insertBefore(enfant, reference) {
            appel("insere", this.__id, identifiantDe(enfant), identifiantDe(reference));
            signale("childList", this, { ajoutes: [enfant] });
            connecte(enfant);
            return enfant;
        }
        removeChild(enfant) {
            appel("retire", identifiantDe(enfant));
            signale("childList", this, { retires: [enfant] });
            deconnecte(enfant);
            return enfant;
        }
        replaceChild(neuf, ancien) {
            appel("insere", this.__id, identifiantDe(neuf), identifiantDe(ancien));
            appel("retire", identifiantDe(ancien));
            signale("childList", this, { ajoutes: [neuf], retires: [ancien] });
            return ancien;
        }
        remove() {
            const parent = this.parentNode;
            appel("retire", this.__id);
            if (parent) signale("childList", parent, { retires: [this] });
            deconnecte(this);
        }
        contains(autre) {
            if (!autre || autre.__id === undefined) return false;
            if (autre.__id === this.__id) return true;
            return !!appel("contient", this.__id, autre.__id);
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
            if (liste.indexOf(fonction) < 0) {
                liste.push(fonction);
                // Compte tenu a jour pour la mesure de compatibilite : c'est le
                // seul indicateur de l'interactivite reelle d'une page qui ne
                // demande pas de la piloter.
                globalThis.__bo_ecouteurs = (globalThis.__bo_ecouteurs || 0) + 1;
                const seau = globalThis.__bo_ecouteurs_types ||
                    (globalThis.__bo_ecouteurs_types = {});
                seau[type] = (seau[type] || 0) + 1;
            }
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
    /// Le formulaire : ce qu'il contient, ou il envoie, et comment.
    ///
    /// `form.elements`, `action`, `method` et `submit()` n'existaient pas. Une
    /// page pouvait afficher un formulaire, pas l'envoyer : tout le chemin
    /// entre « l'utilisateur a rempli » et « le serveur a recu » manquait.
    function installeFormulaire(classe) {
        Object.defineProperty(classe.prototype, "elements", {
            configurable: true,
            get() {
                const liste = noeuds(appel("champsFormulaire", this.__id));
                // Les controles nommes sont aussi accessibles par leur nom,
                // comme `f.elements.utilisateur` — la forme que prend la
                // moitie du code de formulaire du Web.
                for (const controle of liste) {
                    const nom = controle.getAttribute("name");
                    if (nom && !(nom in liste)) liste[nom] = controle;
                }
                return liste;
            },
        });
        Object.defineProperty(classe.prototype, "length", {
            configurable: true,
            get() { return this.elements.length; },
        });
        classe.prototype.submit = function () {
            appel("soumets", this.__id, false);
        };
        classe.prototype.requestSubmit = function () {
            // La difference avec `submit()` : `requestSubmit()` declenche
            // l'evenement `submit`, donc laisse la page annuler. C'est ce que
            // fait un vrai bouton, et `submit()` est justement celui qui ne le
            // fait pas.
            appel("soumets", this.__id, true);
        };
        classe.prototype.reset = function () {
            for (const controle of this.elements) {
                const type = (controle.type || "").toLowerCase();
                if (type === "checkbox" || type === "radio")
                    controle.checked = controle.hasAttribute("checked");
                else if (controle.tagName.toLowerCase() !== "button")
                    controle.value = controle.getAttribute("value") || "";
            }
        };
    }

    /// `element.dataset` : la vue objet des attributs `data-*`.
    ///
    /// Absent, il ne degradait pas — il arretait. `t.dataset.dropdownBound`
    /// leve une `TypeError` sur `undefined`, et le script entier de la page
    /// meurt a cette ligne. C'est ainsi que pypi.org perdait ses menus, ses
    /// onglets et ses dix-huit ecouteurs.
    ///
    /// Un `Proxy` plutot qu'une copie : les attributs changent sous nos pieds
    /// — un autre script, un `setAttribute`, le moteur lui-meme — et une copie
    /// rendrait des valeurs perimees. La conversion de nom suit la norme :
    /// `data-dropdown-bound` devient `dropdownBound` et retour.
    function nomDeDonnee(cle) {
        return "data-" + String(cle).replace(/[A-Z]/g, (c) => "-" + c.toLowerCase());
    }

    function cleDeDonnee(nom) {
        return nom.slice("data-".length)
            .replace(/-([a-z])/g, (_, c) => c.toUpperCase());
    }

    function fabriqueDataset(element) {
        return new Proxy({}, {
            get(_, cle) {
                if (typeof cle !== "string") return undefined;
                const valeur = element.getAttribute(nomDeDonnee(cle));
                return valeur === null ? undefined : valeur;
            },
            set(_, cle, valeur) {
                element.setAttribute(nomDeDonnee(cle), String(valeur));
                return true;
            },
            has(_, cle) {
                return typeof cle === "string" &&
                    element.getAttribute(nomDeDonnee(cle)) !== null;
            },
            deleteProperty(_, cle) {
                element.removeAttribute(nomDeDonnee(cle));
                return true;
            },
            ownKeys() {
                const noms = appel("attributs", element.__id) || {};
                return Object.keys(noms)
                    .filter((n) => n.startsWith("data-"))
                    .map(cleDeDonnee);
            },
            getOwnPropertyDescriptor(_, cle) {
                if (typeof cle !== "string") return undefined;
                const valeur = element.getAttribute(nomDeDonnee(cle));
                if (valeur === null) return undefined;
                return { value: valeur, writable: true, enumerable: true,
                         configurable: true };
            },
        });
    }

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
            const ancienne = appel("attribut", this.__id, String(nom));
            appel("poseAttribut", this.__id, String(nom), String(valeur));
            signale("attributes", this, { attribut: String(nom), ancienne });
            attributChange(this, String(nom), ancienne, String(valeur));
        }
        removeAttribute(nom) {
            const ancienne = appel("attribut", this.__id, String(nom));
            appel("poseAttribut", this.__id, String(nom), null);
            signale("attributes", this, { attribut: String(nom), ancienne });
            attributChange(this, String(nom), ancienne, null);
        }

        /// Ouvre une racine d'ombre sur cet element. Voir [`RacineOmbre`].
        attachShadow(options) {
            if (this.__ombre) return this.__ombre;
            this.__ombre = new RacineOmbre(this, (options && options.mode) || "open");
            return this.__ombre;
        }
        get shadowRoot() { return this.__ombre || null; }
        hasAttribute(nom) { return appel("attribut", this.__id, String(nom)) !== null; }
        get attributes() { return appel("attributs", this.__id); }

        get id() { return this.getAttribute("id") || ""; }
        set id(valeur) { this.setAttribute("id", valeur); }
        get className() { return this.getAttribute("class") || ""; }
        set className(valeur) { this.setAttribute("class", valeur); }
        get classList() { return new ListeClasses(this); }

        get dataset() {
            // Memorise par element : `el.dataset.a = 1; el.dataset.b = 2` doit
            // agir sur le meme objet, et un `Proxy` neuf a chaque lecture
            // fonctionnerait mais couterait pour rien.
            if (!this.__dataset) this.__dataset = fabriqueDataset(this);
            return this.__dataset;
        }

        get innerHTML() { return appel("html", this.__id); }
        set innerHTML(valeur) {
            appel("poseHtml", this.__id, String(valeur));
            signale("childList", this, { ajoutes: this.childNodes });
        }
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
        //
        // Trois elements ne rangent pas leur valeur dans un attribut `value`,
        // et les traiter comme les autres rendait la chaine vide : un `<select>`
        // porte la sienne dans l'option choisie, un `<textarea>` dans son texte,
        // une `<option>` sans attribut vaut son propre libelle. C'est le
        // scenario de connexion qui l'a montre — `espace` revenait vide alors
        // que la page avait bien preselectionne « equipe ».
        get value() {
            const balise = this.tagName.toLowerCase();
            // Un champ editable tient sa valeur dans le modele d'edition, la ou
            // la frappe l'ecrit. La lire dans l'attribut rendrait la valeur par
            // defaut et perdrait tout ce que l'utilisateur a tape.
            if (balise === "input" || balise === "textarea") {
                const courante = appel("valeurChamp", this.__id);
                if (courante !== null && courante !== undefined) return courante;
            }
            if (balise === "select") {
                const choisie = this.__optionChoisie();
                return choisie ? choisie.value : "";
            }
            if (balise === "textarea") {
                return this.hasAttribute("value")
                    ? this.getAttribute("value") : this.textContent;
            }
            if (balise === "option") {
                return this.hasAttribute("value")
                    ? this.getAttribute("value") : (this.textContent || "").trim();
            }
            return this.getAttribute("value") || "";
        }
        set value(v) {
            const balise = this.tagName.toLowerCase();
            if ((balise === "input" || balise === "textarea")
                    && appel("poseValeurChamp", this.__id, String(v)) !== false) {
                return;
            }
            if (balise === "select") {
                // Choisir par valeur, comme le fait `select.value = "x"` :
                // l'option correspondante devient la selectionnee, et aucune
                // autre ne l'est.
                let trouvee = false;
                for (const option of this.__options()) {
                    const correspond = !trouvee && option.value === String(v);
                    if (correspond) trouvee = true;
                    if (correspond) option.setAttribute("selected", "");
                    else option.removeAttribute("selected");
                }
                return;
            }
            this.setAttribute("value", String(v));
        }
        __options() {
            return this.getElementsByTagName
                ? Array.prototype.slice.call(this.querySelectorAll("option"))
                : [];
        }
        __optionChoisie() {
            const options = this.__options();
            for (const option of options)
                if (option.hasAttribute("selected")) return option;
            // Sans attribut `selected`, un `<select>` simple choisit sa premiere
            // option — c'est ce qu'affiche le navigateur, donc ce que le
            // formulaire enverra.
            return options.length && !this.hasAttribute("multiple") ? options[0] : null;
        }
        get options() { return this.__options(); }
        get selectedIndex() {
            const options = this.__options();
            const choisie = this.__optionChoisie();
            return choisie ? options.indexOf(choisie) : -1;
        }
        set selectedIndex(index) {
            const options = this.__options();
            options.forEach((option, rang) => {
                if (rang === Number(index)) option.setAttribute("selected", "");
                else option.removeAttribute("selected");
            });
        }
        get selected() { return this.hasAttribute("selected"); }
        set selected(v) {
            if (v) this.setAttribute("selected", "");
            else this.removeAttribute("selected");
        }
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
        /// Le foyer, pour de vrai.
        ///
        /// C'etait un moignon : la page appelait `focus()`, rien ne se passait,
        /// `document.activeElement` restait `null`, et `:focus` ne peignait
        /// jamais rien. Un formulaire ou l'on ne peut pas designer le champ
        /// courant n'est pas remplissable — c'est la premiere marche de tout
        /// le reste : clavier, `Tab`, accessibilite.
        focus() { appel("poseFoyer", this.__id); }
        blur() {
            // `blur()` ne deplace le foyer que si c'est bien nous qui l'avons :
            // un `blur` appele sur un element quelconque ne doit pas voler le
            // foyer a un autre.
            if (appel("foyer") === this.__id) appel("poseFoyer", null);
        }
        scrollIntoView() { appel("creux", "element.scrollIntoView"); }

        /// Le formulaire qui contient ce controle. `null` hors formulaire.
        get form() { return noeud(appel("formulaireDe", this.__id)); }

        get tabIndex() {
            const brut = this.getAttribute("tabindex");
            if (brut !== null && brut !== "") return parseInt(brut, 10) || 0;
            return ["input", "select", "textarea", "button", "a", "area"]
                .indexOf(this.tagName.toLowerCase()) >= 0 ? 0 : -1;
        }
        set tabIndex(valeur) { this.setAttribute("tabindex", String(valeur)); }

        get required() { return this.hasAttribute("required"); }
        set required(v) {
            if (v) this.setAttribute("required", "");
            else this.removeAttribute("required");
        }
        get readOnly() { return this.hasAttribute("readonly"); }
        set readOnly(v) {
            if (v) this.setAttribute("readonly", "");
            else this.removeAttribute("readonly");
        }
        get placeholder() { return this.getAttribute("placeholder") || ""; }
        set placeholder(v) { this.setAttribute("placeholder", String(v)); }
        get name() { return this.getAttribute("name") || ""; }
        set name(v) { this.setAttribute("name", String(v)); }
        get type() {
            const balise = this.tagName.toLowerCase();
            if (balise === "input") return (this.getAttribute("type") || "text").toLowerCase();
            if (balise === "button") return (this.getAttribute("type") || "submit").toLowerCase();
            return balise;
        }
        set type(v) { this.setAttribute("type", String(v)); }

        /// `required` non rempli : la seule regle de validation qui compte
        /// aujourd'hui. Le reste de la validation HTML5 — motifs, bornes,
        /// types — attendra que celle-ci serve reellement.
        ///
        /// Un `<form>` valide tous ses controles ; tout le reste se valide
        /// lui-meme. Poser deux `checkValidity` distincts, l'un sur le
        /// formulaire et l'autre sur les champs, faisait que le premier
        /// installe ecrasait le second — et chaque champ repondait alors comme
        /// un formulaire vide, c'est-a-dire toujours valide.
        checkValidity() {
            if (this.tagName.toLowerCase() === "form") {
                return this.elements.every(
                    (c) => typeof c.checkValidity !== "function" || c.checkValidity());
            }
            if (!this.required) return true;
            if (this.type === "checkbox" || this.type === "radio")
                return !!this.checked;
            return String(this.value || "").length > 0;
        }
        reportValidity() { return this.checkValidity(); }
        get validity() {
            const rempli = this.checkValidity();
            return { valid: rempli, valueMissing: !rempli, customError: false,
                     typeMismatch: false, patternMismatch: false };
        }
        setCustomValidity() {}

        /// Selection minimale dans un champ. Le curseur n'a pas de rendu, mais
        /// `selectionStart` est lu par toute bibliotheque de masque de saisie,
        /// et rendre `undefined` la faisait lever.
        get selectionStart() {
            const s = appel("selectionChamp", this.__id);
            return s ? s.debut : null;
        }
        set selectionStart(v) {
            const s = appel("selectionChamp", this.__id) || { fin: 0 };
            appel("poseSelectionChamp", this.__id, Number(v) || 0, s.fin);
        }
        get selectionEnd() {
            const s = appel("selectionChamp", this.__id);
            return s ? s.fin : null;
        }
        set selectionEnd(v) {
            const s = appel("selectionChamp", this.__id) || { debut: 0 };
            appel("poseSelectionChamp", this.__id, s.debut, Number(v) || 0);
        }
        setSelectionRange(debut, fin) {
            appel("poseSelectionChamp", this.__id, Number(debut) || 0,
                  Number(fin) || 0);
        }
        select() {
            appel("poseSelectionChamp", this.__id, 0,
                  String(this.value || "").length);
            this.focus();
        }
        click() { distribue(this, new Event("click", { bubbles: true, cancelable: true })); }
    }

    // --- Elements media -----------------------------------------------------
    //
    // `<video>` et `<audio>` ne different que par ce qu'ils montrent : meme
    // etat, memes commandes, memes evenements. Une seule classe les couvre donc,
    // et Python decide s'il y a une image a peindre.

    // --- Toile (canvas 2D) ----------------------------------------------------
    //
    // Le contexte n'a pas de pixels : il **enregistre** ce qu'on lui demande de
    // dessiner, dans le meme vocabulaire que la liste d'affichage du moteur —
    // rectangles, lignes, textes, images. La mise en page pose ensuite ces
    // operations a l'emplacement de la toile, et l'hote Qt les peint comme le
    // reste de la page.
    //
    // Ce que cela donne : les graphiques, les jauges, les frises et les petits
    // jeux s'affichent reellement — y compris les degrades, les ombres, le
    // detourage et les chemins remplis exactement, chacun ayant son operation
    // dans la liste d'affichage.
    //
    // La lecture des pixels marche aussi, mais differemment : `getImageData`
    // demande a l'hote de **jouer** les operations dans une image hors ecran et
    // d'en rendre les octets. C'est le meme peintre que l'ecran, donc ce qu'on
    // lit est ce qu'on voit. Ce qui reste dehors : les motifs
    // (`createPattern`) et le mode de composition.

    function nombre(valeur) {
        const n = Number(valeur);
        return Number.isFinite(n) ? n : 0;
    }

    class ContexteToile {
        constructor(toile) {
            this.canvas = toile;
            this.__operations = [];
            this.__chemin = [];
            this.__pile = [];
            this.__dx = 0;
            this.__dy = 0;
            this.__rognages = 0;
            this.__envoiPrevu = false;
            this.fillStyle = "#000000";
            this.strokeStyle = "#000000";
            this.lineWidth = 1;
            this.font = "10px sans-serif";
            this.textAlign = "left";
            this.textBaseline = "alphabetic";
            this.globalAlpha = 1;
            this.shadowColor = "transparent";
            this.shadowBlur = 0;
            this.shadowOffsetX = 0;
            this.shadowOffsetY = 0;
        }

        // --- Etat ---
        save() {
            this.__pile.push({
                fillStyle: this.fillStyle, strokeStyle: this.strokeStyle,
                lineWidth: this.lineWidth, font: this.font,
                textAlign: this.textAlign, dx: this.__dx, dy: this.__dy,
                shadowColor: this.shadowColor, shadowBlur: this.shadowBlur,
                shadowOffsetX: this.shadowOffsetX,
                shadowOffsetY: this.shadowOffsetY,
                // Un `clip()` posee apres ce `save()` doit tomber avec lui :
                // sans ce compte, le detourage survivrait au `restore()` et
                // rognerait tout le reste du dessin.
                rognages: this.__rognages,
            });
        }
        restore() {
            const etat = this.__pile.pop();
            if (!etat) return;
            this.fillStyle = etat.fillStyle;
            this.strokeStyle = etat.strokeStyle;
            this.lineWidth = etat.lineWidth;
            this.font = etat.font;
            this.textAlign = etat.textAlign;
            this.__dx = etat.dx;
            this.__dy = etat.dy;
            this.shadowColor = etat.shadowColor;
            this.shadowBlur = etat.shadowBlur;
            this.shadowOffsetX = etat.shadowOffsetX;
            this.shadowOffsetY = etat.shadowOffsetY;
            while (this.__rognages > etat.rognages) {
                this.__operations.push(["declip"]);
                this.__rognages--;
            }
            this.__envoie();
        }
        translate(x, y) { this.__dx += nombre(x); this.__dy += nombre(y); }
        setTransform(a, b, c, d, e, f) { this.__dx = nombre(e); this.__dy = nombre(f); }
        resetTransform() { this.__dx = 0; this.__dy = 0; }
        scale() {}
        rotate() {}

        // --- Rectangles ---
        fillRect(x, y, l, h) {
            const gx = nombre(x) + this.__dx;
            const gy = nombre(y) + this.__dy;
            const gl = nombre(l);
            const gh = nombre(h);
            this.__ombre(gx, gy, gl, gh);
            const pente = this.__pente(this.fillStyle, gx, gy, gl, gh);
            if (pente) { this.__pose(pente); return; }
            this.__pose(["rect", gx, gy, gl, gh, String(this.fillStyle)]);
        }
        strokeRect(x, y, l, h) {
            const gx = nombre(x) + this.__dx;
            const gy = nombre(y) + this.__dy;
            const gl = nombre(l);
            const gh = nombre(h);
            const e = nombre(this.lineWidth) || 1;
            const teinte = String(this.strokeStyle);
            this.__pose(["rect", gx, gy, gl, e, teinte]);
            this.__pose(["rect", gx, gy + gh - e, gl, e, teinte]);
            this.__pose(["rect", gx, gy, e, gh, teinte]);
            this.__pose(["rect", gx + gl - e, gy, e, gh, teinte]);
        }
        clearRect(x, y, l, h) {
            // Effacer toute la toile est la facon dont on redessine une trame :
            // on repart de rien, ce qui est exactement ce que veut l'appelant.
            // Un effacement partiel, faute de tampon de pixels, se rend par un
            // rectangle blanc — approximatif, mais visible plutot que faux.
            const toute = nombre(x) + this.__dx <= 0 && nombre(y) + this.__dy <= 0
                && nombre(l) >= this.canvas.width && nombre(h) >= this.canvas.height;
            if (toute) {
                this.__operations = [];
                this.__envoie();
                return;
            }
            this.__pose(["rect", nombre(x) + this.__dx, nombre(y) + this.__dy,
                         nombre(l), nombre(h), "#ffffff"]);
        }

        // --- Chemins ---
        beginPath() { this.__chemin = []; }
        closePath() {
            if (this.__chemin.length > 1) this.__chemin.push(this.__chemin[0]);
        }
        moveTo(x, y) {
            this.__chemin.push([nombre(x) + this.__dx, nombre(y) + this.__dy]);
        }
        lineTo(x, y) {
            this.__chemin.push([nombre(x) + this.__dx, nombre(y) + this.__dy]);
        }
        rect(x, y, l, h) {
            const gx = nombre(x) + this.__dx;
            const gy = nombre(y) + this.__dy;
            this.__chemin.push([gx, gy], [gx + nombre(l), gy],
                               [gx + nombre(l), gy + nombre(h)],
                               [gx, gy + nombre(h)], [gx, gy]);
        }
        arc(cx, cy, rayon, debut, fin) {
            // Un arc devient une ligne brisee : a l'echelle d'un ecran, seize
            // segments par tour ne se distinguent pas d'un cercle.
            const centreX = nombre(cx) + this.__dx;
            const centreY = nombre(cy) + this.__dy;
            const r = nombre(rayon);
            const a0 = nombre(debut);
            const a1 = fin === undefined ? Math.PI * 2 : nombre(fin);
            const pas = Math.max(8, Math.round(Math.abs(a1 - a0) / (Math.PI / 8)));
            for (let i = 0; i <= pas; i++) {
                const angle = a0 + (a1 - a0) * (i / pas);
                this.__chemin.push([centreX + r * Math.cos(angle),
                                    centreY + r * Math.sin(angle)]);
            }
        }
        stroke() {
            const e = nombre(this.lineWidth) || 1;
            for (let i = 1; i < this.__chemin.length; i++) {
                const a = this.__chemin[i - 1];
                const b = this.__chemin[i];
                this.__pose(["ligne", a[0], a[1], b[0], b[1], e,
                             String(this.strokeStyle)]);
            }
        }
        fill() {
            if (this.__chemin.length < 2) return;
            const b = this.__boite();
            this.__ombre(b.x, b.y, b.l, b.h);
            // Un degrade se rend sur la boite du chemin : l'hote sait remplir un
            // rectangle d'un degrade, pas un polygone.
            const pente = this.__pente(this.fillStyle, b.x, b.y, b.l, b.h);
            if (pente) { this.__pose(pente); return; }
            if (this.__chemin.length >= 3) {
                // Le chemin est rempli tel quel. Sa boite englobante suffisait
                // pour un `rect()` et faisait d'un camembert un carre.
                this.__pose(["polygone", this.__chemin.map((p) => [p[0], p[1]]),
                             String(this.fillStyle)]);
                return;
            }
            this.__pose(["rect", b.x, b.y, b.l, b.h, String(this.fillStyle)]);
        }

        // --- Texte ---
        fillText(texte, x, y) {
            const police = this.__police();
            this.__pose(["texte", nombre(x) + this.__dx,
                         nombre(y) + this.__dy - police.taille,
                         String(texte), String(this.fillStyle), police.taille,
                         police.gras, police.italique, police.fixe]);
        }
        strokeText(texte, x, y) { this.fillText(texte, x, y); }
        measureText(texte) {
            const police = this.__police();
            return {
                width: appel("largeurTexte", String(texte), police.taille,
                             police.gras, police.fixe),
                actualBoundingBoxAscent: police.taille * 0.8,
                actualBoundingBoxDescent: police.taille * 0.2,
            };
        }

        // --- Images ---
        drawImage(source, x, y, l, h) {
            const identifiant = source && (source.__image !== undefined
                ? source.__image : source.__id);
            if (identifiant === undefined || identifiant === null) return;
            this.__pose(["image", nombre(x) + this.__dx, nombre(y) + this.__dy,
                         l === undefined ? nombre(source.width) : nombre(l),
                         h === undefined ? nombre(source.height) : nombre(h),
                         identifiant]);
        }

        // --- Degrades ---
        createLinearGradient(x0, y0, x1, y1) {
            return new DegradeToile("lineaire", [nombre(x0), nombre(y0),
                                                 nombre(x1), nombre(y1)]);
        }
        createRadialGradient(x0, y0, r0, x1, y1, r1) {
            // Le cercle interieur n'est pas rendu : l'hote n'a qu'un centre et
            // un rayon, et c'est le cercle exterieur qui dessine le degrade.
            return new DegradeToile("radial", [nombre(x1), nombre(y1), nombre(r1)]);
        }

        // --- Detourage ---
        clip() {
            if (this.__chemin.length < 2) return;
            const b = this.__boite();
            // Le detourage suit la boite du chemin : l'hote empile des
            // rectangles, pas des formes. Un `clip()` apres `rect()` — de loin
            // le cas le plus courant — est donc exact.
            this.__operations.push(["clip", b.x, b.y, b.l, b.h]);
            this.__rognages++;
            this.__envoie();
        }

        // --- Pixels ---
        getImageData(x, y, l, h) {
            const largeur = Math.max(1, Math.round(nombre(this.canvas.width)));
            const hauteur = Math.max(1, Math.round(nombre(this.canvas.height)));
            const tout = appel("rasterise", this.__operations, largeur, hauteur);
            const rx = Math.max(0, Math.round(nombre(x)));
            const ry = Math.max(0, Math.round(nombre(y)));
            const rl = Math.max(0, Math.min(Math.round(nombre(l)), largeur - rx));
            const rh = Math.max(0, Math.min(Math.round(nombre(h)), hauteur - ry));
            const donnees = new Uint8ClampedArray(rl * rh * 4);
            if (tout) {
                for (let j = 0; j < rh; j++) {
                    const source = ((ry + j) * largeur + rx) * 4;
                    const cible = j * rl * 4;
                    for (let i = 0; i < rl * 4; i++) {
                        donnees[cible + i] = tout[source + i];
                    }
                }
            }
            return { data: donnees, width: rl, height: rh };
        }
        putImageData(image, x, y) {
            if (!image || !image.data) return;
            const identifiant = appel("imageBrute", Array.from(image.data),
                                      nombre(image.width), nombre(image.height));
            if (identifiant === null || identifiant === undefined) return;
            this.__pose(["image", nombre(x) + this.__dx, nombre(y) + this.__dy,
                         nombre(image.width), nombre(image.height), identifiant]);
        }
        createImageData(l, h) {
            const largeur = Math.max(0, Math.round(nombre(l)));
            const hauteur = Math.max(0, Math.round(nombre(h)));
            return { data: new Uint8ClampedArray(largeur * hauteur * 4),
                     width: largeur, height: hauteur };
        }

        // --- Ce qui n'est pas tenu, mais ne doit pas faire tomber la page ---
        createPattern() { return null; }
        setLineDash() {}
        getLineDash() { return []; }
        quadraticCurveTo(_, __, x, y) { this.lineTo(x, y); }
        bezierCurveTo(_, __, ___, ____, x, y) { this.lineTo(x, y); }

        // --- Rouages ---
        __boite() {
            let x0 = this.__chemin[0][0], y0 = this.__chemin[0][1];
            let x1 = x0, y1 = y0;
            for (const [x, y] of this.__chemin) {
                if (x < x0) x0 = x;
                if (y < y0) y0 = y;
                if (x > x1) x1 = x;
                if (y > y1) y1 = y;
            }
            return { x: x0, y: y0, l: x1 - x0, h: y1 - y0 };
        }

        __pente(style, x, y, l, h) {
            if (!(style instanceof DegradeToile) || style.__arrets.length < 2) {
                return null;
            }
            if (style.__genre === "radial") {
                const [cx, cy, r] = style.__geometrie;
                return ["degrade_cercle", x, y, l, h,
                        cx + this.__dx, cy + this.__dy, r, style.__arrets];
            }
            const [x0, y0, x1, y1] = style.__geometrie;
            return ["degrade_points", x, y, l, h,
                    x0 + this.__dx, y0 + this.__dy,
                    x1 + this.__dx, y1 + this.__dy, style.__arrets];
        }

        __ombre(x, y, l, h) {
            const flou = nombre(this.shadowBlur);
            const dx = nombre(this.shadowOffsetX);
            const dy = nombre(this.shadowOffsetY);
            const teinte = String(this.shadowColor || "transparent");
            if (teinte === "transparent" || (!flou && !dx && !dy)) return;
            this.__operations.push(["ombre", x + dx, y + dy, l, h, 0, flou, teinte]);
        }

        __police() {
            const police = String(this.font);
            const trouve = police.match(/(\d+(?:\.\d+)?)px/);
            return {
                taille: trouve ? parseFloat(trouve[1]) : 10,
                gras: /bold|[6-9]00/.test(police),
                italique: /italic/.test(police),
                fixe: /mono/.test(police),
            };
        }

        __pose(operation) {
            this.__operations.push(operation);
            this.__envoie();
        }

        __envoie() {
            // Les operations partent en un seul envoi, a la fin de la tache en
            // cours : une trame de jeu en fait des centaines, et les envoyer une
            // par une couterait plus que tout le dessin.
            if (this.__envoiPrevu) return;
            this.__envoiPrevu = true;
            globalThis.queueMicrotask(() => {
                this.__envoiPrevu = false;
                appel("toile", this.canvas.__id, this.__operations);
            });
        }
    }

    class DegradeToile {
        constructor(genre, geometrie) {
            this.__genre = genre;
            this.__geometrie = geometrie;
            this.__arrets = [];
        }
        addColorStop(position, couleur) {
            this.__arrets.push([nombre(position), String(couleur)]);
            this.__arrets.sort((a, b) => a[0] - b[0]);
        }
    }

    class ElementToile extends Element {
        get width() { return parseInt(this.getAttribute("width") || "300", 10); }
        set width(valeur) { this.setAttribute("width", String(valeur)); }
        get height() { return parseInt(this.getAttribute("height") || "150", 10); }
        set height(valeur) { this.setAttribute("height", String(valeur)); }
        getContext(type) {
            if (String(type) !== "2d") return null;
            if (!this.__contexte) this.__contexte = new ContexteToile(this);
            return this.__contexte;
        }
        toDataURL() { return "data:,"; }
    }

    globalThis.HTMLCanvasElement = ElementToile;
    globalThis.CanvasRenderingContext2D = ContexteToile;
    globalThis.CanvasGradient = DegradeToile;

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

    // `action` et `method` doivent etre resolus, pas rendus bruts : une page
    // qui lit `form.action` attend une adresse absolue, et un `method` en
    // minuscules — c'est ce que la norme garantit et ce dont depend tout code
    // qui reconstruit la requete lui-meme.
    Object.defineProperty(Element.prototype, "action", {
        configurable: true,
        get() {
            if (this.tagName.toLowerCase() !== "form") return this.getAttribute("action");
            const brut = this.getAttribute("action");
            return brut ? appel("resout", brut) : appel("url");
        },
        set(v) { this.setAttribute("action", String(v)); },
    });
    Object.defineProperty(Element.prototype, "method", {
        configurable: true,
        get() {
            if (this.tagName.toLowerCase() !== "form") return this.getAttribute("method");
            return (this.getAttribute("method") || "get").toLowerCase();
        },
        set(v) { this.setAttribute("method", String(v)); },
    });
    Object.defineProperty(Element.prototype, "htmlFor", {
        configurable: true,
        get() { return this.getAttribute("for") || ""; },
        set(v) { this.setAttribute("for", String(v)); },
    });
    Object.defineProperty(Element.prototype, "control", {
        configurable: true,
        get() {
            // `label.control` : le controle que cette etiquette designe, soit
            // par `for`, soit parce qu'il est a l'interieur. C'est ce lien qui
            // fait qu'un clic sur le libelle donne le foyer au champ.
            if (this.tagName.toLowerCase() !== "label") return null;
            const cible = this.getAttribute("for");
            if (cible) return document.getElementById(cible);
            return this.querySelector("input, select, textarea, button");
        },
    });
    installeFormulaire(Element);

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

    /// Appelle un rappel d'observateur sans laisser son erreur remonter.
    ///
    /// `invoque` ne convient pas : il ne passe qu'un argument et nomme
    /// l'evenement dans son message. Les observateurs, eux, recoivent leur lot
    /// **et** eux-memes en second.
    function appelleSur(fonction, cible) {
        const arguments_ = Array.prototype.slice.call(arguments, 2);
        try {
            fonction.apply(cible, arguments_);
        } catch (e) {
            console.error("erreur dans un observateur :", e);
        }
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
        abort() { appel("creux", "XMLHttpRequest.abort"); }
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

    /// `AbortSignal` / `AbortController` — annuler une requete en vol.
    ///
    /// Poser `window.AbortController = function () {}` aurait fait reussir la
    /// detection de fonctionnalite et laisse la page croire qu'elle sait
    /// annuler. Ici le signal porte reellement l'annulation jusqu'au fil
    /// reseau : la requete n'est pas interrompue au milieu — on ne va pas
    /// fermer une prise depuis un autre fil — mais sa reponse ne sera jamais
    /// livree, et la promesse est rejetee avec un `AbortError`. Du point de vue
    /// de la page, c'est exactement une annulation.
    class AbortSignal {
        constructor() {
            this.aborted = false;
            this.reason = undefined;
            this.__ecouteurs = new Map();
            this.onabort = null;
        }
        addEventListener(type, f, o) {
            Nœud.prototype.addEventListener.call(this, type, f, o);
        }
        removeEventListener(type, f, o) {
            Nœud.prototype.removeEventListener.call(this, type, f, o);
        }
        throwIfAborted() { if (this.aborted) throw this.reason; }
        __declenche(raison) {
            if (this.aborted) return;
            this.aborted = true;
            this.reason = raison;
            const evenement = new Event("abort");
            for (const f of ecouteursDe(this, "abort", false))
                invoque(f, this, evenement);
            if (typeof this.onabort === "function") this.onabort(evenement);
        }
    }

    function erreurAnnulation(raison) {
        if (raison !== undefined) return raison;
        const e = new Error("The operation was aborted.");
        e.name = "AbortError";
        return e;
    }

    class AbortController {
        constructor() { this.signal = new AbortSignal(); }
        abort(raison) { this.signal.__declenche(erreurAnnulation(raison)); }
    }

    globalThis.AbortSignal = AbortSignal;
    globalThis.AbortController = AbortController;

    globalThis.fetch = function (url, options) {
        options = options || {};
        const signal = options.signal;
        return new Promise(function (resoud, rejette) {
            if (signal && signal.aborted) {
                // Deja annule avant de partir : aucune requete n'est emise.
                rejette(erreurAnnulation(signal.reason));
                return;
            }
            const identifiant = prochaineRequete++;
            let fini = false;
            requetes.set(identifiant, function (reponse) {
                if (fini) return;
                fini = true;
                if (reponse.status > 0) resoud(new Reponse(reponse));
                else rejette(new TypeError("echec du chargement : " + url));
            });
            if (signal && typeof signal.addEventListener === "function") {
                signal.addEventListener("abort", function () {
                    if (fini) return;
                    fini = true;
                    requetes.delete(identifiant);
                    appel("annuleRequete", identifiant);
                    rejette(erreurAnnulation(signal.reason));
                });
            }
            let corps = options.body;
            if (corps === undefined || corps === null) corps = null;
            else if (typeof corps !== "string") corps = String(corps);
            appel("requete", identifiant,
                  String(options.method || "GET").toUpperCase(), String(url),
                  corps, options.headers || {}, false);
        });
    };

    // --- Document et fenetre ------------------------------------------------

    class Document extends Nœud {
        constructor() { super(appel("racine")); this.readyState = "loading"; }

        /// `document.activeElement` — l'element qui recevra la prochaine
        /// frappe. Il rendait `null` en toutes circonstances, ce qui rend
        /// impossible tout pilotage au clavier et toute gestion de foyer.
        ///
        /// La norme veut `body` quand rien n'a le foyer, jamais `null` sur un
        /// document charge : du code ecrit `document.activeElement.tagName`
        /// sans garde, et `null` le tue.
        get activeElement() {
            return noeud(appel("foyer")) || this.body || null;
        }
        hasFocus() { return true; }
        get documentElement() { return noeud(appel("racine")); }
        get body() { return noeud(appel("corps")); }
        get head() { return noeud(appel("tete")); }
        get title() { return appel("titre"); }
        set title(valeur) { appel("poseTitre", String(valeur)); }
        get URL() { return appel("url"); }
        get location() { return globalThis.location; }
        // Les temoins sont ceux du bocal persistant, pas une chaine vide :
        // une page qui lit `document.cookie` pour savoir si elle a une session
        // en tirait la conclusion inverse de la verite.
        get cookie() { return appel("temoins") || ""; }
        set cookie(declaration) { appel("poseTemoin", String(declaration)); }

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

    // --- Composants (Custom Elements) -----------------------------------------
    //
    // Une bibliotheque de composants declare ses balises, et le navigateur
    // s'engage a instancier la classe correspondante pour chaque element qui
    // porte cette balise — a l'analyse comme plus tard. Sans cela, une page
    // faite de `<mon-menu>` et `<ma-carte>` affiche des balises inconnues, donc
    // rien.
    //
    // La mise a niveau se fait presque toujours a la creation de l'enveloppe
    // (voir `noeud`), ce qui evite d'avoir a remplacer un objet que du code
    // tient peut-etre deja. Il ne reste a rattraper que les elements dont
    // l'enveloppe existait avant la declaration.

    function construitComposant(definition, identifiant) {
        const precedent = enConstruction;
        enConstruction = identifiant;
        try {
            const objet = new definition.classe();
            objet.__id = identifiant;
            return objet;
        } catch (e) {
            console.error("composant :", e);
            return new Element(identifiant);
        } finally {
            enConstruction = precedent;
        }
    }

    /// Rattrape un element dont l'enveloppe existait deja.
    function ameliore(element) {
        if (!element || element.__composant) return element;
        const definition = definitions.get(element.localName);
        if (!definition) return element;
        const neuf = construitComposant(definition, element.__id);
        neuf.__composant = true;
        // Les ecouteurs poses sur l'ancienne enveloppe suivent : les perdre
        // rendrait un composant sourd des l'instant ou on le declare.
        neuf.__ecouteurs = element.__ecouteurs;
        if (element.__ombre) neuf.__ombre = element.__ombre;
        enveloppes.set(element.__id, neuf);
        return neuf;
    }

    function connecte(nœud) {
        const objet = enveloppes.get(nœud && nœud.__id);
        if (!objet || typeof objet.connectedCallback !== "function") return;
        if (objet.__connecte) return;
        objet.__connecte = true;
        appelleSur(objet.connectedCallback, objet);
    }

    function deconnecte(nœud) {
        const objet = enveloppes.get(nœud && nœud.__id);
        if (!objet || !objet.__connecte) return;
        objet.__connecte = false;
        if (typeof objet.disconnectedCallback === "function")
            appelleSur(objet.disconnectedCallback, objet);
    }

    function attributChange(nœud, nom, ancienne, nouvelle) {
        const objet = enveloppes.get(nœud && nœud.__id);
        if (!objet || typeof objet.attributeChangedCallback !== "function") return;
        const observes = objet.constructor && objet.constructor.observedAttributes;
        if (Array.isArray(observes) && observes.indexOf(nom) < 0) return;
        appelleSur(objet.attributeChangedCallback, objet, nom, ancienne, nouvelle);
    }

    const attentes = new Map();

    class RegistreComposants {
        define(nom, classe) {
            nom = String(nom).toLowerCase();
            if (definitions.has(nom))
                throw new Error("le composant " + nom + " est deja declare");
            if (nom.indexOf("-") < 0)
                throw new Error("un nom de composant doit contenir un tiret : " + nom);
            definitions.set(nom, { classe });

            // Ce qui est deja dans l'arbre est mis a niveau puis connecte, dans
            // l'ordre du document : un composant parent doit exister avant que
            // son enfant ne cherche a l'atteindre.
            for (const element of document.getElementsByTagName(nom)) {
                const objet = ameliore(element);
                connecte(objet);
            }

            const attendus = attentes.get(nom);
            if (attendus) {
                attentes.delete(nom);
                for (const resoud of attendus) resoud(classe);
            }
        }
        get(nom) {
            const definition = definitions.get(String(nom).toLowerCase());
            return definition ? definition.classe : undefined;
        }
        getName(classe) {
            for (const [nom, definition] of definitions)
                if (definition.classe === classe) return nom;
            return null;
        }
        upgrade(nœud) { ameliore(nœud); }
        whenDefined(nom) {
            nom = String(nom).toLowerCase();
            const definition = definitions.get(nom);
            if (definition) return Promise.resolve(definition.classe);
            return new Promise((resoud) => {
                if (!attentes.has(nom)) attentes.set(nom, []);
                attentes.get(nom).push(resoud);
            });
        }
    }

    globalThis.customElements = new RegistreComposants();

    // --- Racine d'ombre -------------------------------------------------------
    //
    // Le DOM d'ombre isole le contenu d'un composant du reste de la page. Ici,
    // l'isolement n'est pas tenu : la racine d'ombre est un element reel,
    // `<bo-ombre>`, insere dans l'hote. Ce que cela donne — et c'est le point —
    // c'est que le contenu d'un composant **s'affiche**, ce qui est la
    // difference entre une page et une page blanche.
    //
    // Ce que cela ne donne pas : les selecteurs de la page atteignent le
    // contenu d'ombre, et `:host` comme `<slot>` ne sont pas interpretes. Une
    // page qui compte sur l'isolement verra ses regles fuir dans les deux sens.

    class RacineOmbre {
        constructor(hote, mode) {
            const support = document.createElement("bo-ombre");
            hote.appendChild(support);
            this.__support = support;
            this.__id = support.__id;
            this.host = hote;
            this.mode = mode;
        }
        get innerHTML() { return this.__support.innerHTML; }
        set innerHTML(valeur) { this.__support.innerHTML = valeur; }
        get textContent() { return this.__support.textContent; }
        set textContent(valeur) { this.__support.textContent = valeur; }
        get childNodes() { return this.__support.childNodes; }
        get children() { return this.__support.children; }
        get firstChild() { return this.__support.firstChild; }
        get lastChild() { return this.__support.lastChild; }
        get activeElement() {
            // Dans une racine d'ombre, `activeElement` ne designe le foyer que
            // s'il se trouve a l'interieur : c'est tout l'interet de
            // l'encapsulation, et rendre le foyer du document ferait fuir une
            // information que l'ombre est censee cacher.
            const actuel = noeud(appel("foyer"));
            if (!actuel) return null;
            return this.__support && this.__support.contains(actuel) ? actuel : null;
        }
        appendChild(enfant) { return this.__support.appendChild(enfant); }
        insertBefore(enfant, avant) { return this.__support.insertBefore(enfant, avant); }
        removeChild(enfant) { return this.__support.removeChild(enfant); }
        querySelector(selecteur) { return this.__support.querySelector(selecteur); }
        querySelectorAll(selecteur) { return this.__support.querySelectorAll(selecteur); }
        getElementById(identifiant) {
            return this.__support.querySelector("#" + String(identifiant));
        }
        addEventListener(type, f, o) { this.__support.addEventListener(type, f, o); }
        removeEventListener(type, f, o) { this.__support.removeEventListener(type, f, o); }
        dispatchEvent(e) { return this.__support.dispatchEvent(e); }
    }

    globalThis.ShadowRoot = RacineOmbre;

    /// `window.location`, en lecture ; naviguer se demande a l'hote.
    const location = {
        get href() { return appel("url"); },
        set href(valeur) { appel("navigue", String(valeur)); },
        assign(valeur) { appel("navigue", String(valeur)); },
        replace(valeur) { appel("navigue", String(valeur)); },
        reload() { appel("navigue", appel("url")); },
        toString() { return appel("url"); },
    };
    Object.defineProperty(location, "hash", {
        get() { return appel("urlPartie", "hash"); },
        set(valeur) {
            // Changer le fragment ne recharge pas le document : cela ajoute une
            // entree d'historique et emet `hashchange`. Le confondre avec une
            // navigation rechargeait la page a chaque ancre cliquee.
            let fragment = String(valeur);
            if (fragment && fragment[0] !== "#") fragment = "#" + fragment;
            const avant = appel("url");
            appel("poseEtat", null, "", fragment || "#", false);
            const apres = appel("url");
            if (avant !== apres) {
                const evenement = new Event("hashchange");
                evenement.oldURL = avant;
                evenement.newURL = apres;
                globalThis.dispatchEvent(evenement);
            }
        },
        enumerable: true,
        configurable: true,
    });
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
    /// `localStorage` : cloisonne par origine, ecrit sur le disque.
    ///
    /// Une page range la son etat — le theme choisi, un panier, un brouillon —
    /// et s'attend a le retrouver au retour. Une table en memoire, comme
    /// auparavant, ne survivait meme pas au changement de page.
    const stockageLocal = {
        getItem(cle) {
            const valeur = appel("stockage", "lit", String(cle));
            return valeur === undefined ? null : valeur;
        },
        setItem(cle, valeur) { appel("stockage", "ecrit", String(cle), String(valeur)); },
        removeItem(cle) { appel("stockage", "retire", String(cle)); },
        clear() { appel("stockage", "efface"); },
        key(index) { return (appel("stockage", "cles") || [])[index] || null; },
        get length() { return (appel("stockage", "cles") || []).length; },
    };

    /// `sessionStorage` ne survit pas a la page : c'est sa definition.
    function stockageDeSession() {
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

    globalThis.localStorage = stockageLocal;
    globalThis.sessionStorage = stockageDeSession();
    globalThis.Storage = Object;

    // `navigator` porte une trentaine de champs dont la plupart ne servent a
    // rien — sauf qu'un script les lit sans les tester. `appVersion` en est
    // l'exemple exact : `navigator.appVersion.includes(…)` est la ligne qui
    // tuait tout le JavaScript de pypi.org, et avec lui les dix-huit ecouteurs
    // que la page voulait poser. Un champ absent ne degrade pas, il arrete.
    const AGENT = "Mozilla/5.0 (Bouchaud OS; x86_64) BoNavigateur/1.0";
    globalThis.navigator = {
        userAgent: AGENT,
        // `appVersion` est l'agent utilisateur ampute de son prefixe
        // « Mozilla/ », ce que tout navigateur rend depuis trente ans.
        appVersion: AGENT.slice("Mozilla/".length),
        appName: "Netscape",
        appCodeName: "Mozilla",
        product: "Gecko",
        productSub: "20030107",
        vendor: "",
        vendorSub: "",
        platform: "BouchaudOS",
        oscpu: "BouchaudOS x86_64",
        language: "fr-FR",
        languages: ["fr-FR", "fr", "en"],
        onLine: true,
        cookieEnabled: true,
        doNotTrack: null,
        webdriver: false,
        maxTouchPoints: 0,
        hardwareConcurrency: 1,
        pdfViewerEnabled: false,
        plugins: [],
        mimeTypes: [],
        userAgentData: undefined,
        javaEnabled: function () { return false; },
        taintEnabled: function () { return false; },
    };

    const taille = appel("tailleVue") || { width: 1280, height: 720 };
    globalThis.screen = {
        width: taille.width, height: taille.height,
        availWidth: taille.width, availHeight: taille.height,
        colorDepth: 32, pixelDepth: 32,
    };

    /// `window.history` — l'historique de session du document.
    ///
    /// `pushState` et `replaceState` etaient vides. Une application a page
    /// unique appelait, rien ne bougeait, et `location.pathname` rendait
    /// eternellement l'adresse de depart : barre d'adresse figee, bouton
    /// « precedent » sans effet, aucun `popstate`. C'est-a-dire, en pratique,
    /// une forme entiere du Web applicatif inutilisable.
    ///
    /// Le troisieme argument est facultatif et peut valoir `null` ou `""`, ce
    /// qui veut dire « garde l'adresse actuelle » — a ne pas confondre avec
    /// « va a la racine ».
    globalThis.history = {
        get length() { return appel("longueurHistorique"); },
        get state() { return appel("etatHistorique"); },
        scrollRestoration: "auto",
        pushState(etat, titre, url) {
            appel("poseEtat", etat === undefined ? null : etat,
                  titre === undefined ? "" : titre,
                  url === undefined || url === null ? null : String(url), false);
        },
        replaceState(etat, titre, url) {
            appel("poseEtat", etat === undefined ? null : etat,
                  titre === undefined ? "" : titre,
                  url === undefined || url === null ? null : String(url), true);
        },
        back() { appel("historique", -1); },
        forward() { appel("historique", 1); },
        go(n) { appel("historique", Number(n) || 0); },
    };

    /// Appele par Python quand un deplacement a change l'entree courante.
    /// L'evenement porte l'etat memorise, comme le veut la norme — c'est lui
    /// qui permet a l'application de retrouver la vue qu'elle affichait.
    globalThis.__bo_popstate = function (etat) {
        const evenement = new Event("popstate");
        evenement.state = etat === undefined ? null : etat;
        globalThis.dispatchEvent(evenement);
        if (typeof globalThis.onpopstate === "function")
            globalThis.onpopstate(evenement);
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
    globalThis.pageXOffset = 0;
    // La position de defilement est celle du navigateur, pas une constante :
    // une page qui la lit pour decider quoi afficher — une barre qui se colle
    // en haut, un chargement paresseux — depend entierement de sa justesse.
    Object.defineProperty(globalThis, "scrollY", { get: () => appel("defilement") });
    Object.defineProperty(globalThis, "pageYOffset", { get: () => appel("defilement") });

    globalThis.alert = function (message) { appel("console", "alerte", formate(message)); };
    globalThis.confirm = function () { return false; };
    globalThis.prompt = function () { return null; };
    globalThis.scrollTo = creux("window.scrollTo");
    globalThis.scrollBy = creux("window.scrollBy");
    globalThis.open = creux("window.open", null);
    globalThis.close = creux("window.close");
    globalThis.focus = creux("window.focus");
    globalThis.blur = creux("window.blur");
    globalThis.matchMedia = function (requete) {
        return { matches: false, media: String(requete),
                 addListener() {}, removeListener() {},
                 addEventListener() {}, removeEventListener() {} };
    };
    // `getComputedStyle` rend le style **resolu** : cascade, heritage et style
    // en ligne reunis. Rendre `element.style`, comme on le faisait, ne montrait
    // que l'attribut `style` — une page qui lit une couleur de theme ou une
    // hauteur posee par une feuille recevait une chaine vide et se croyait sur
    // une page sans mise en forme.
    class StyleCalcule {
        constructor(proprietes) { this.__p = proprietes || {}; }
        getPropertyValue(nom) { return this.__p[String(nom)] || ""; }
        getPropertyPriority() { return ""; }
        get length() { return Object.keys(this.__p).length; }
        item(index) { return Object.keys(this.__p)[index] || ""; }
        get cssText() {
            return Object.keys(this.__p).map((n) => n + ": " + this.__p[n]).join("; ");
        }
    }

    globalThis.CSSStyleDeclaration = StyleCalcule;

    globalThis.getComputedStyle = function (element) {
        if (!element || element.__id === undefined) return new StyleCalcule({});
        const proprietes = appel("styleCalcule", element.__id) || {};
        return new Proxy(new StyleCalcule(proprietes), {
            get(cible, propriete) {
                if (propriete in cible) return cible[propriete];
                if (typeof propriete !== "string") return undefined;
                return cible.getPropertyValue(tiret(propriete));
            },
        });
    };

    // --- Observateurs ---------------------------------------------------------
    //
    // `MutationObserver` est ce sur quoi reposent les bibliotheques de
    // composants pour savoir qu'on leur a ajoute un nœud. Il est tenu ici, dans
    // le prelude, parce que **toute** modification de l'arbre y passe : les
    // methodes du DOM sont les seules portes vers Python, il suffit donc de les
    // faire declarer ce qu'elles changent. Aucun code Python n'a besoin de le
    // savoir.

    const observateurs = [];
    let livraisonProgrammee = false;

    function signale(type, cible, details) {
        if (observateurs.length === 0) return;
        const enregistrement = {
            type,
            target: cible,
            addedNodes: (details && details.ajoutes) || [],
            removedNodes: (details && details.retires) || [],
            attributeName: (details && details.attribut) || null,
            oldValue: (details && details.ancienne) !== undefined
                ? details.ancienne : null,
            previousSibling: null,
            nextSibling: null,
            attributeNamespace: null,
        };
        for (const observateur of observateurs)
            if (observateur.__concerne(type, cible))
                observateur.__file.push(enregistrement);
        if (!livraisonProgrammee) {
            livraisonProgrammee = true;
            // Les enregistrements sont livres en micro-tache, comme le veut la
            // norme : un script qui fait dix ajouts de suite recoit un seul
            // appel portant les dix, et non dix appels.
            globalThis.queueMicrotask(livre);
        }
    }

    function livre() {
        livraisonProgrammee = false;
        for (const observateur of observateurs.slice()) {
            if (observateur.__file.length === 0) continue;
            const lot = observateur.__file;
            observateur.__file = [];
            appelleSur(observateur.__rappel, observateur, lot, observateur);
        }
    }

    class MutationObserver {
        constructor(rappel) {
            this.__rappel = rappel;
            this.__cibles = [];
            this.__file = [];
        }
        observe(cible, options) {
            options = options || {};
            this.__cibles.push({
                nœud: cible,
                enfants: !!options.childList,
                attributs: !!options.attributes,
                donnees: !!options.characterData,
                sous_arbre: !!options.subtree,
            });
            if (observateurs.indexOf(this) < 0) observateurs.push(this);
        }
        disconnect() {
            this.__cibles = [];
            this.__file = [];
            const position = observateurs.indexOf(this);
            if (position >= 0) observateurs.splice(position, 1);
        }
        takeRecords() {
            const lot = this.__file;
            this.__file = [];
            return lot;
        }
        __concerne(type, cible) {
            for (const entree of this.__cibles) {
                if (type === "childList" && !entree.enfants) continue;
                if (type === "attributes" && !entree.attributs) continue;
                if (type === "characterData" && !entree.donnees) continue;
                if (entree.nœud.__id === cible.__id) return true;
                if (entree.sous_arbre && descendDe(cible, entree.nœud)) return true;
            }
            return false;
        }
    }

    function descendDe(nœud, ancetre) {
        if (!nœud || !ancetre || nœud.__id === ancetre.__id) return false;
        return !!appel("contient", ancetre.__id, nœud.__id);
    }

    globalThis.MutationObserver = MutationObserver;
    globalThis.__bo_signale_mutation = signale;

    // `IntersectionObserver` sert au chargement paresseux des images et aux
    // animations d'apparition. Il n'a pas de mecanisme propre : il compare, a
    // chaque battement, la boite de l'element a la fenetre. C'est exactement ce
    // que fait un navigateur, a ceci pres qu'il le fait sur changement plutot
    // qu'a intervalle.
    const guetteurs = [];

    class IntersectionObserver {
        constructor(rappel, options) {
            this.__rappel = rappel;
            this.__cibles = [];
            this.__etats = new Map();
            this.root = (options && options.root) || null;
            this.rootMargin = (options && options.rootMargin) || "0px";
            const seuils = (options && options.threshold);
            this.thresholds = Array.isArray(seuils) ? seuils : [seuils || 0];
        }
        observe(cible) {
            if (this.__cibles.indexOf(cible) < 0) this.__cibles.push(cible);
            if (guetteurs.indexOf(this) < 0) guetteurs.push(this);
            // Un premier examen immediat : une image deja visible au chargement
            // doit se declarer visible sans attendre un defilement.
            this.__examine();
        }
        unobserve(cible) {
            const position = this.__cibles.indexOf(cible);
            if (position >= 0) this.__cibles.splice(position, 1);
            this.__etats.delete(cible);
        }
        disconnect() {
            this.__cibles = [];
            this.__etats.clear();
            const position = guetteurs.indexOf(this);
            if (position >= 0) guetteurs.splice(position, 1);
        }
        takeRecords() { return []; }
        __examine() {
            const defilement = appel("defilement") || 0;
            const vue = appel("tailleVue") || { width: 0, height: 0 };
            const entrees = [];
            for (const cible of this.__cibles) {
                const boite = cible.getBoundingClientRect();
                const haut = boite.y - defilement;
                const bas = haut + boite.height;
                const visible = bas > 0 && haut < vue.height
                    && boite.width > 0 && boite.height > 0;
                if (this.__etats.get(cible) === visible) continue;
                this.__etats.set(cible, visible);
                const hauteurVisible = Math.max(
                    0, Math.min(bas, vue.height) - Math.max(haut, 0));
                entrees.push({
                    target: cible,
                    isIntersecting: visible,
                    intersectionRatio: boite.height > 0
                        ? hauteurVisible / boite.height : 0,
                    boundingClientRect: { x: boite.x, y: haut, width: boite.width,
                                          height: boite.height, top: haut,
                                          left: boite.x, right: boite.x + boite.width,
                                          bottom: bas },
                    rootBounds: { x: 0, y: 0, width: vue.width, height: vue.height,
                                  top: 0, left: 0, right: vue.width,
                                  bottom: vue.height },
                    intersectionRect: { x: boite.x, y: Math.max(haut, 0),
                                        width: boite.width, height: hauteurVisible },
                    time: Date.now(),
                });
            }
            if (entrees.length > 0) appelleSur(this.__rappel, this, entrees, this);
        }
    }

    globalThis.IntersectionObserver = IntersectionObserver;

    // Appele par Python a chaque battement, apres la mise en page : c'est le
    // seul moment ou les boites sont a jour.
    globalThis.__bo_guette = function () {
        for (const guetteur of guetteurs.slice()) {
            try { guetteur.__examine(); } catch (e) { console.error(e); }
        }
    };

    // `ResizeObserver` se ramene au meme examen : la taille d'une boite ne
    // change qu'a la mise en page.
    class ResizeObserver {
        constructor(rappel) { this.__rappel = rappel; this.__cibles = []; this.__tailles = new Map(); }
        observe(cible) {
            if (this.__cibles.indexOf(cible) < 0) this.__cibles.push(cible);
            if (redimensionnes.indexOf(this) < 0) redimensionnes.push(this);
            this.__examine();
        }
        unobserve(cible) {
            const position = this.__cibles.indexOf(cible);
            if (position >= 0) this.__cibles.splice(position, 1);
        }
        disconnect() {
            this.__cibles = [];
            const position = redimensionnes.indexOf(this);
            if (position >= 0) redimensionnes.splice(position, 1);
        }
        __examine() {
            const entrees = [];
            for (const cible of this.__cibles) {
                const boite = cible.getBoundingClientRect();
                const cle = boite.width + "x" + boite.height;
                if (this.__tailles.get(cible) === cle) continue;
                this.__tailles.set(cible, cle);
                entrees.push({
                    target: cible,
                    contentRect: boite,
                    borderBoxSize: [{ inlineSize: boite.width, blockSize: boite.height }],
                    contentBoxSize: [{ inlineSize: boite.width, blockSize: boite.height }],
                });
            }
            if (entrees.length > 0) appelleSur(this.__rappel, this, entrees, this);
        }
    }

    const redimensionnes = [];
    globalThis.ResizeObserver = ResizeObserver;
    globalThis.__bo_redimensionne = function () {
        for (const observateur of redimensionnes.slice()) {
            try { observateur.__examine(); } catch (e) { console.error(e); }
        }
    };
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


    // --- Formulaires --------------------------------------------------------
    //
    // `new FormData(formulaire)` est la facon dont un site envoie un
    // formulaire sans recharger la page. Sans elle, le code de soumission
    // leve, et c'est tout l'envoi qui tombe — pas seulement l'encodage.
    //
    // Les regles d'inclusion sont celles de la norme, et elles comptent :
    // un champ sans `name` n'est pas envoye, une case non cochee non plus,
    // un champ desactive non plus. Les inclure ferait recevoir au serveur des
    // valeurs que l'utilisateur n'a pas donnees.

    function champsDuFormulaire(formulaire) {
        if (!formulaire || !formulaire.querySelectorAll) return [];
        const champs = formulaire.querySelectorAll("input, select, textarea");
        const retenus = [];
        for (const champ of champs) {
            const nom = champ.getAttribute("name");
            if (!nom || champ.disabled) continue;
            const type = (champ.getAttribute("type") || "").toLowerCase();
            if (type === "submit" || type === "button" || type === "reset"
                    || type === "image" || type === "file") continue;
            if ((type === "checkbox" || type === "radio") && !champ.checked) continue;
            let valeur = champ.value;
            if ((type === "checkbox" || type === "radio") && valeur === "")
                valeur = "on";
            retenus.push([nom, valeur]);
        }
        return retenus;
    }

    class FormData {
        constructor(formulaire) {
            this.__paires = formulaire ? champsDuFormulaire(formulaire) : [];
        }
        append(nom, valeur) { this.__paires.push([String(nom), String(valeur)]); }
        set(nom, valeur) {
            this.delete(nom);
            this.append(nom, valeur);
        }
        get(nom) {
            for (const [cle, valeur] of this.__paires)
                if (cle === String(nom)) return valeur;
            return null;
        }
        getAll(nom) {
            return this.__paires.filter(([cle]) => cle === String(nom))
                .map(([, valeur]) => valeur);
        }
        has(nom) { return this.get(nom) !== null; }
        delete(nom) {
            this.__paires = this.__paires.filter(([cle]) => cle !== String(nom));
        }
        forEach(fonction, contexte) {
            for (const [cle, valeur] of this.__paires.slice())
                fonction.call(contexte, valeur, cle, this);
        }
        *entries() { for (const paire of this.__paires.slice()) yield paire; }
        *keys() { for (const [cle] of this.__paires.slice()) yield cle; }
        *values() { for (const [, valeur] of this.__paires.slice()) yield valeur; }
        [Symbol.iterator]() { return this.entries(); }
        toString() {
            // Ce que `fetch(url, {body: formData})` envoie faute de contenu
            // multipart : la meme chose qu'un formulaire ordinaire.
            return this.__paires
                .map(([c, v]) => encodeURIComponent(c) + "=" + encodeURIComponent(v))
                .join("&");
        }
    }

    globalThis.FormData = FormData;

    globalThis.URLSearchParams = class URLSearchParams extends FormData {
        constructor(source) {
            super(null);
            const texte = String(source || "").replace(/^[?]/, "");
            if (!texte) return;
            for (const morceau of texte.split("&")) {
                if (!morceau) continue;
                const coupe = morceau.indexOf("=");
                const cle = coupe < 0 ? morceau : morceau.slice(0, coupe);
                const valeur = coupe < 0 ? "" : morceau.slice(coupe + 1);
                this.__paires.push([decodeURIComponent(cle.replace(/\+/g, " ")),
                                    decodeURIComponent(valeur.replace(/\+/g, " "))]);
            }
        }
    };

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
            appel("ouvreSocket", this.__id, this.url, liste);
        }

        addEventListener(type, f, o) {
            Nœud.prototype.addEventListener.call(this, type, f, o);
        }
        removeEventListener(type, f, o) {
            Nœud.prototype.removeEventListener.call(this, type, f, o);
        }

        send(donnees) {
            if (this.readyState !== 1)
                throw new Error("WebSocket: la connexion n'est pas ouverte");
            appel("envoieSocket", this.__id,
                  typeof donnees === "string" ? donnees : Array.from(donnees));
        }

        close(code, raison) {
            if (this.readyState === 2 || this.readyState === 3) return;
            this.readyState = 2;              // CLOSING
            appel("fermeSocket", this.__id, code === undefined ? 1000 : code,
                  raison === undefined ? "" : String(raison));
        }

        __recoit(type, charge) {
            let evenement;
            if (type === "message") {
                evenement = new Event("message");
                evenement.data = charge;
                evenement.origin = this.url;
                this.readyState = 1;
            } else if (type === "open") {
                this.readyState = 1;
                evenement = new Event("open");
            } else if (type === "close") {
                this.readyState = 3;          // CLOSED
                evenement = new Event("close");
                evenement.code = (charge && charge.code) || 1006;
                evenement.reason = (charge && charge.raison) || "";
                evenement.wasClean = !!(charge && charge.propre);
                sockets.delete(this.__id);
            } else {
                evenement = new Event("error");
                evenement.message = String(charge || "");
            }
            for (const f of ecouteursDe(this, evenement.type, false))
                invoque(f, this, evenement);
            const propre = "on" + evenement.type;
            if (typeof this[propre] === "function") {
                try { this[propre](evenement); }
                catch (e) { console.error(e); }
            }
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

    // --- Mesure de compatibilite --------------------------------------------
    //
    // Le moteur ne sait pas tout faire, et jusqu'ici il ne savait pas dire ce
    // qui lui manquait : une page cassee laissait deviner. Ce bloc installe,
    // sur chaque nom de la plate-forme que le moteur ne fournit pas, un
    // accesseur qui **note l'acces et rend `undefined`**.
    //
    // Rendre `undefined` n'est pas un detail. Poser un objet factice ferait
    // reussir `if (window.WebSocket)` et la page prendrait le chemin qu'elle
    // n'aurait pas du prendre — la mesure changerait le comportement mesure.
    // Ici le script voit exactement ce qu'il voyait avant, la detection de
    // fonctionnalite continue de retomber sur sa solution de repli, et le
    // rapport apprend qu'on lui a demande quelque chose.
    //
    // La liste ne se veut pas exhaustive : ce qu'elle rate finit dans les
    // erreurs JavaScript, qui sont notees aussi.

    // Un moignon signale son propre vide. Le nom part au rapport, la valeur
    // rendue ne change pas — la page continue exactement comme avant.
    //
    // Pourquoi les compter separement des APIs absentes : une page qui teste
    // `if (history.pushState)` obtient `true`, prend le chemin moderne, et
    // poursuit avec un navigateur dont l'etat ne correspond plus a ce qu'elle
    // croit. Une absence l'aurait fait retomber sur son plan de secours. Le
    // moignon est donc souvent le plus couteux des deux, et le plus invisible.
    function creux(nom, rendu) {
        return function () {
            appel("creux", nom);
            return rendu;
        };
    }

    const NOMS_FENETRE = [
        "WebSocket", "EventSource", "Worker", "SharedWorker", "ServiceWorker",
        "indexedDB", "IDBFactory", "IDBKeyRange", "caches", "CacheStorage",
        "BroadcastChannel", "MessageChannel", "MessagePort", "SharedArrayBuffer",
        "Notification", "PushManager", "RTCPeerConnection", "WebGLRenderingContext",
        "WebGL2RenderingContext", "OffscreenCanvas", "createImageBitmap",
        "ImageBitmap", "ImageData", "Path2D",
        "FileReader", "Blob", "File", "FileList", "FormData",
        "URLSearchParams", "AbortController", "AbortSignal",
        "Headers", "Request", "Response",
        "ReadableStream", "WritableStream", "TransformStream", "TextEncoderStream",
        "structuredClone", "requestIdleCallback", "cancelIdleCallback",
        "reportError", "queueMicrotask",
        "IntersectionObserver", "PerformanceObserver", "ReportingObserver",
        "crypto", "TextEncoder", "TextDecoder", "Intl",
        "matchMedia", "getSelection", "getComputedStyle",
        "DOMParser", "XSLTProcessor", "XPathEvaluator", "Range", "Selection",
        "NodeFilter", "TreeWalker", "NodeIterator",
        "performance", "screen", "visualViewport", "scrollTo", "scrollBy",
        "open", "close", "print", "alert", "confirm", "prompt",
        "postMessage", "focus", "blur", "getScreenDetails",
        "CSS", "CSSStyleSheet", "FontFace", "speechSynthesis",
        "customElements", "HTMLTemplateElement", "DocumentFragment",
        "trustedTypes", "navigation", "scheduler",
    ];

    const MEMBRES = [
        ["Element", "closest matches webkitMatchesSelector insertAdjacentHTML " +
            "insertAdjacentElement insertAdjacentText scrollIntoView scrollIntoViewIfNeeded " +
            "getBoundingClientRect getClientRects animate attachShadow " +
            "replaceChildren replaceWith toggleAttribute requestFullscreen " +
            "setPointerCapture releasePointerCapture computedStyleMap " +
            "checkVisibility scrollTo scrollBy"],
        ["Node", "compareDocumentPosition isEqualNode isSameNode normalize " +
            "getRootNode lookupNamespaceURI"],
        ["Event", "composedPath stopImmediatePropagation"],
    ];

    function surveille(objet, nom, chemin) {
        let deja;
        try { deja = objet[nom]; } catch (e) { return; }
        if (deja !== undefined) return;
        try {
            Object.defineProperty(objet, nom, {
                configurable: true,
                get: function () { appel("manque", chemin); return undefined; },
                set: function (v) {
                    // Une page qui installe sa propre implementation — un
                    // « polyfill » — a le droit de le faire : on s'efface.
                    Object.defineProperty(objet, nom, {
                        configurable: true, writable: true,
                        enumerable: true, value: v,
                    });
                },
            });
        } catch (e) { /* non configurable : rien a surveiller */ }
    }

    for (const nom of NOMS_FENETRE) surveille(globalThis, nom, nom);
    for (const [porteur, liste] of MEMBRES) {
        const cible = globalThis[porteur];
        if (!cible || !cible.prototype) continue;
        for (const nom of liste.split(" ")) {
            if (nom) surveille(cible.prototype, nom, porteur + ".prototype." + nom);
        }
    }
    for (const [objet, prefixe] of [[globalThis.navigator, "navigator"],
                                    [globalThis.history, "history"],
                                    [globalThis.location, "location"]]) {
        if (!objet) continue;
        const noms = prefixe === "navigator"
            ? ["clipboard", "geolocation", "mediaDevices", "serviceWorker",
               "permissions", "credentials", "storage", "share", "sendBeacon",
               "connection", "deviceMemory", "locks", "usb", "bluetooth",
               "wakeLock", "xr", "gpu", "presentation", "scheduling",
               "registerProtocolHandler", "getBattery", "vibrate", "buildID"]
            : prefixe === "history"
                ? ["pushState", "replaceState", "state", "scrollRestoration", "go"]
                : ["replace", "assign", "reload", "ancestorOrigins"];
        for (const nom of noms) surveille(objet, nom, prefixe + "." + nom);
    }

    // Appele par Python avant d'envoyer un formulaire : la page a le droit
    // d'annuler. Rend `false` si elle l'a fait.
    globalThis.__bo_soumission = function (identifiant) {
        const formulaire = noeud(identifiant);
        if (!formulaire) return false;
        const evenement = new Event("submit", { cancelable: true });
        evenement.submitter = null;
        return formulaire.dispatchEvent(evenement);
    };

    // Les paires `nom=valeur` qu'un formulaire enverrait. Memes regles
    // d'inclusion que `FormData` — un seul endroit ou elles sont ecrites,
    // sinon la soumission et `FormData` finiraient par diverger.
    globalThis.__bo_champsSoumis = function (identifiant) {
        const formulaire = noeud(identifiant);
        return formulaire ? champsDuFormulaire(formulaire) : [];
    };

    // Etat du document, mis a jour par Python quand l'analyse est finie.
    globalThis.__bo_pret = function () {
        document.readyState = "interactive";
        globalThis.__bo_evenement(null, "DOMContentLoaded", {});
        document.readyState = "complete";
        globalThis.__bo_evenement(null, "load", {});
    };
})();
