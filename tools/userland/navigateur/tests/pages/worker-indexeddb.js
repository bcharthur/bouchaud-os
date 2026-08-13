// IndexedDB depuis un Worker.
//
// La base est **la meme** que celle du document : le service est cloisonne par
// origine, et un worker partage l'origine de la page qui l'a cree. C'est ce que
// dit la norme, et c'est ce qui rend possible le cas d'usage le plus courant —
// un worker qui synchronise en fond ce que la page a ecrit devant.
//
// Ce fichier ecrit deux enregistrements, en relit un, compte, puis rend la
// main. La page verifiera ensuite qu'elle voit ce que le worker a ecrit.

function ouvre() {
    return new Promise(function (resoudre, rejeter) {
        const demande = indexedDB.open("bo-worker", 1);
        demande.onupgradeneeded = function () {
            demande.result.createObjectStore("notes");
        };
        demande.onsuccess = function () { resoudre(demande.result); };
        demande.onerror = function () { rejeter(demande.error); };
    });
}

function attend(requete) {
    return new Promise(function (resoudre, rejeter) {
        requete.onsuccess = function () { resoudre(requete.result); };
        requete.onerror = function () { rejeter(requete.error); };
    });
}

self.onmessage = function (evenement) {
    if ((evenement.data || {}).genre !== "va") return;
    const rapport = { genre: "rapport" };

    ouvre().then(function (base) {
        rapport.version = base.version;
        rapport.magasins = Array.from(base.objectStoreNames);

        const ecriture = base.transaction("notes", "readwrite");
        const magasin = ecriture.objectStore("notes");
        magasin.put({ texte: "ecrit par le worker", n: 1 }, "a");
        magasin.put({ texte: "et un second", n: 2 }, "b");

        return new Promise(function (resoudre, rejeter) {
            ecriture.oncomplete = function () { resoudre(base); };
            ecriture.onabort = function () { rejeter(new Error("abandonnee")); };
        });
    }).then(function (base) {
        const lecture = base.transaction("notes", "readonly");
        const magasin = lecture.objectStore("notes");
        return Promise.all([attend(magasin.get("a")), attend(magasin.count()),
                            attend(magasin.getAllKeys())]);
    }).then(function (resultats) {
        rapport.lu = resultats[0];
        rapport.combien = resultats[1];
        rapport.cles = resultats[2];
        // Une cle absente doit rendre `undefined`, pas `null` : c'est la
        // distinction sur laquelle une page decide s'il faut creer
        // l'enregistrement.
        return ouvre();
    }).then(function (base) {
        const t = base.transaction("notes", "readonly");
        return attend(t.objectStore("notes").get("jamais-ecrite"));
    }).then(function (valeur) {
        rapport.absenteEstIndefinie = valeur === undefined;
        rapport.fait = true;
        postMessage(rapport);
    }).catch(function (e) {
        rapport.erreur = String((e && e.message) || e);
        postMessage(rapport);
    });
};
