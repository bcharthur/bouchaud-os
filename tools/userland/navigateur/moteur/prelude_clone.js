// Le clonage structure : une seule facon de faire traverser une valeur.
//
// ## Le probleme qu'il resout
//
// Quatre chemins font passer des donnees d'un monde a un autre :
// `Worker.postMessage`, `Window.postMessage`, `MessagePort.postMessage` et
// IndexedDB. Tous les quatre s'appuyaient sur la conversion du pont Python, qui
// est fidele pour les nombres, les chaines, les tableaux et les objets simples
// — et **silencieusement fausse** pour tout le reste :
//
//   * `undefined` devenait `null`. Une page qui distingue « pas de valeur » de
//     « valeur nulle » — c'est-a-dire toute page qui teste `=== undefined` —
//     recevait la mauvaise reponse ;
//   * une fonction devenait `null` au lieu de lever ;
//   * un `Map`, un `Set`, une `Date`, un `RegExp` devenaient `{}` : la forme
//     survivait, le contenu disparaissait ;
//   * un objet cyclique etait tronque a la trente-deuxieme profondeur, sans
//     que personne n'en soit averti.
//
// Aucun de ces defauts ne se signale. Ils se decouvrent des mois plus tard,
// dans une page qui « perd » un champ, et on cherche du cote de la page.
//
// ## La regle
//
// **Ce qui n'est pas clonable leve `DataCloneError`.** Jamais de conversion
// muette. C'est ce que fait la norme, et c'est ce qui rend le defaut visible a
// l'endroit ou il est commis plutot qu'a l'endroit ou il fait mal.
//
// ## La forme sur le fil
//
// Un paquet est `{ v: 1, r: <index de la racine>, n: [<noeuds>] }`. Chaque
// valeur — y compris les scalaires — est un nœud du tableau, et toute reference
// entre nœuds est un index. Deux consequences :
//
//   * **les cycles traversent.** Un objet deja rencontre rend son index ; a la
//     relecture, la coquille existe avant qu'on la remplisse ;
//   * **le partage est preserve.** Deux champs qui pointaient le meme objet le
//     pointent encore de l'autre cote, au lieu d'en recevoir deux copies.
//
// Le paquet ne contient que des nombres, des chaines, des booleens, des
// tableaux et des objets simples : exactement ce que le pont Python transporte
// sans rien perdre. `NaN` et les infinis survivent — ils traversent en
// `float` — et c'est verifie.
//
// ## Ce qui n'est pas fait
//
// Le transfert (`transfer`) n'existe pas : tout est copie. Un `ArrayBuffer`
// transmis reste utilisable chez l'emetteur, la ou la norme le detacherait.
// C'est une difference observable, et elle est dite ici plutot que decouverte.

(function () {
    "use strict";

    const VERSION = 1;

    class DataCloneError extends Error {
        constructor(message) {
            super(message);
            this.name = "DataCloneError";
        }
    }

    // Les constructeurs dont une instance ne traverse pas. Chaque prelude
    // remplit cette liste avec les siens : la fenetre y met le DOM, le Worker
    // n'a rien a y mettre. Le module ne connait donc pas le DOM, ce qui est ce
    // qui lui permet de servir aux deux.
    const refuses = [];

    function refuse(constructeur, nom) {
        if (typeof constructeur === "function")
            refuses.push([constructeur, String(nom || constructeur.name)]);
    }

    const OCTETS = {
        Int8Array: 1, Uint8Array: 1, Uint8ClampedArray: 1,
        Int16Array: 2, Uint16Array: 2,
        Int32Array: 4, Uint32Array: 4,
        Float32Array: 4, Float64Array: 8,
    };

    function nomDeVue(valeur) {
        for (const nom of Object.keys(OCTETS))
            if (typeof globalThis[nom] === "function"
                && valeur instanceof globalThis[nom])
                return nom;
        return null;
    }

    // --- Serialisation -------------------------------------------------------

    function serialise(valeur) {
        const noeuds = [];
        const vus = new Map();
        const racine = pose(valeur, noeuds, vus);
        return { v: VERSION, r: racine, n: noeuds };
    }

    function ajoute(noeuds, nœud) {
        noeuds.push(nœud);
        return noeuds.length - 1;
    }

    function pose(valeur, noeuds, vus) {
        if (valeur === undefined) return ajoute(noeuds, ["u"]);
        if (valeur === null) return ajoute(noeuds, ["z"]);

        const genre = typeof valeur;
        if (genre === "boolean") return ajoute(noeuds, ["b", valeur]);
        if (genre === "number") return ajoute(noeuds, ["d", valeur]);
        if (genre === "string") return ajoute(noeuds, ["s", valeur]);
        if (genre === "bigint") return ajoute(noeuds, ["g", valeur.toString()]);
        if (genre === "symbol")
            throw new DataCloneError("un Symbol ne se clone pas");
        if (genre === "function")
            throw new DataCloneError("une fonction ne se clone pas");

        if (vus.has(valeur)) return vus.get(valeur);

        for (const [constructeur, nom] of refuses) {
            if (valeur instanceof constructeur)
                throw new DataCloneError("un " + nom + " ne se clone pas");
        }

        if (valeur instanceof Date) {
            const rang = ajoute(noeuds, ["t", valeur.getTime()]);
            vus.set(valeur, rang);
            return rang;
        }
        if (valeur instanceof RegExp) {
            const rang = ajoute(noeuds, ["x", valeur.source, valeur.flags]);
            vus.set(valeur, rang);
            return rang;
        }
        if (valeur instanceof Error) {
            const rang = ajoute(noeuds, ["e", String(valeur.name || "Error"),
                                         String(valeur.message || "")]);
            vus.set(valeur, rang);
            return rang;
        }
        if (typeof ArrayBuffer === "function" && valeur instanceof ArrayBuffer) {
            const rang = ajoute(noeuds, ["m", Array.from(new Uint8Array(valeur))]);
            vus.set(valeur, rang);
            return rang;
        }

        const vue = nomDeVue(valeur);
        if (vue) {
            // Le tampon est pose **avant** la vue, pour que son index soit le
            // plus petit : la relecture fabrique les coquilles dans l'ordre des
            // index, et une vue a besoin de son tampon deja construit.
            const tampon = pose(valeur.buffer, noeuds, vus);
            const rang = ajoute(noeuds, ["y", vue, tampon, valeur.byteOffset,
                                         valeur.length]);
            vus.set(valeur, rang);
            return rang;
        }

        if (Array.isArray(valeur)) {
            const rang = ajoute(noeuds, ["a", null]);
            vus.set(valeur, rang);
            const elements = [];
            for (const element of valeur)
                elements.push(pose(element, noeuds, vus));
            noeuds[rang][1] = elements;
            return rang;
        }

        if (typeof Map === "function" && valeur instanceof Map) {
            const rang = ajoute(noeuds, ["p", null]);
            vus.set(valeur, rang);
            const paires = [];
            valeur.forEach(function (v, k) {
                paires.push([pose(k, noeuds, vus), pose(v, noeuds, vus)]);
            });
            noeuds[rang][1] = paires;
            return rang;
        }

        if (typeof Set === "function" && valeur instanceof Set) {
            const rang = ajoute(noeuds, ["q", null]);
            vus.set(valeur, rang);
            const elements = [];
            valeur.forEach(function (v) { elements.push(pose(v, noeuds, vus)); });
            noeuds[rang][1] = elements;
            return rang;
        }

        // Tout le reste est traite comme un objet simple : ses proprietes
        // propres et enumerables traversent, son prototype non. C'est ce que
        // fait la norme d'une instance de classe, et c'est pourquoi les objets
        // qui ne doivent **pas** traverser sont refuses nommement plus haut
        // plutot que devines ici.
        const rang = ajoute(noeuds, ["o", null]);
        vus.set(valeur, rang);
        const champs = [];
        for (const cle of Object.keys(valeur))
            champs.push([cle, pose(valeur[cle], noeuds, vus)]);
        noeuds[rang][1] = champs;
        return rang;
    }

    // --- Relecture -----------------------------------------------------------

    function materialise(paquet) {
        if (!paquet || typeof paquet !== "object" || paquet.v !== VERSION
            || !Array.isArray(paquet.n))
            throw new DataCloneError("paquet de clonage illisible");
        const noeuds = paquet.n;
        const faits = new Array(noeuds.length);

        // Deux passes. La premiere fabrique les coquilles, la seconde les
        // remplit : sans cela un objet qui se contient lui-meme n'aurait rien a
        // quoi se rattacher au moment ou on l'y range.
        for (let i = 0; i < noeuds.length; i++)
            faits[i] = coquille(noeuds[i], faits);
        for (let i = 0; i < noeuds.length; i++)
            remplit(noeuds[i], faits[i], faits);

        return faits[paquet.r];
    }

    function coquille(nœud, faits) {
        switch (nœud[0]) {
        case "u": return undefined;
        case "z": return null;
        case "b": return !!nœud[1];
        case "d": return nœud[1];
        case "s": return String(nœud[1]);
        case "g": return typeof BigInt === "function" ? BigInt(nœud[1]) : nœud[1];
        case "t": return new Date(nœud[1]);
        case "x": return new RegExp(nœud[1], nœud[2]);
        case "e": {
            const erreur = new Error(nœud[2]);
            erreur.name = nœud[1];
            return erreur;
        }
        case "m": {
            const tampon = new ArrayBuffer(nœud[1].length);
            new Uint8Array(tampon).set(nœud[1]);
            return tampon;
        }
        case "y": {
            const Vue = globalThis[nœud[1]];
            return new Vue(faits[nœud[2]], nœud[3], nœud[4]);
        }
        case "a": return [];
        case "p": return new Map();
        case "q": return new Set();
        case "o": return {};
        default:
            throw new DataCloneError("etiquette de clonage inconnue : " + nœud[0]);
        }
    }

    function remplit(nœud, valeur, faits) {
        switch (nœud[0]) {
        case "a":
            for (const rang of nœud[1]) valeur.push(faits[rang]);
            break;
        case "o":
            for (const [cle, rang] of nœud[1]) valeur[cle] = faits[rang];
            break;
        case "p":
            for (const [cle, rang] of nœud[1])
                valeur.set(faits[cle], faits[rang]);
            break;
        case "q":
            for (const rang of nœud[1]) valeur.add(faits[rang]);
            break;
        default:
            break;
        }
    }

    globalThis.__bo_clone = {
        serialise: serialise,
        materialise: materialise,
        refuse: refuse,
        DataCloneError: DataCloneError,
    };
    globalThis.DataCloneError = DataCloneError;

    // `structuredClone()` de la plate-forme : un aller-retour complet, donc la
    // meme definition de ce qui se clone et de ce qui leve. Une page qui s'en
    // sert obtient exactement ce qu'un `postMessage` lui aurait donne.
    globalThis.structuredClone = function (valeur) {
        return materialise(serialise(valeur));
    };
})();
