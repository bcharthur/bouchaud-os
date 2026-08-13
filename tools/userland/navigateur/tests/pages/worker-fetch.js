// `fetch` depuis un Worker.
//
// Le point qui compte n'est pas que `fetch` existe : c'est qu'il passe par la
// meme infrastructure que la fenetre. Meme groupe de fils, meme resolution
// d'adresse relative, meme CORS, meme `Origin`. Une seconde implementation
// aurait fini par diverger sur un detail de securite — et c'est toujours le
// detail de securite qui diverge en dernier, donc le plus tard decouvert.
//
// Ce fichier fait donc trois requetes qui touchent trois comportements
// differents du reseau, et rapporte ce qu'il en obtient.

self.onmessage = function (evenement) {
    const message = evenement.data || {};
    if (message.genre !== "va") return;

    const rapport = { genre: "rapport" };

    // 1. Une adresse relative : elle doit se resoudre contre l'adresse du
    //    *script du worker*, pas contre celle du document.
    fetch("../json")
        .then(function (reponse) {
            rapport.relatifStatut = reponse.status;
            rapport.relatifUrl = reponse.url;
            return reponse.json();
        })
        .then(function (charge) {
            rapport.relatifNom = charge.nom;
            rapport.relatifImbrique = charge.imbrique && charge.imbrique.a;

            // 2. Une redirection : le reseau la suit, et l'adresse finale
            //    remonte jusqu'ici.
            return fetch("/redirect?to=/json&n=2");
        })
        .then(function (reponse) {
            rapport.redirigeStatut = reponse.status;
            rapport.redirigeUrl = reponse.url;

            // 3. Un POST avec un corps : la methode et le corps traversent.
            return fetch("/echo", {
                method: "POST",
                body: "depuis-le-worker",
                headers: { "Content-Type": "text/plain" },
            });
        })
        .then(function (reponse) { return reponse.json(); })
        .then(function (charge) {
            rapport.echoMethode = charge.methode;
            rapport.echoCorps = charge.corps;
            // Un worker n'est pas cross-origine par rapport a son document :
            // la requete ne doit donc **pas** porter d'en-tete `Origin`.
            rapport.echoOrigin = charge.entetes.origin || "";
            rapport.fait = true;
            postMessage(rapport);
        })
        .catch(function (e) {
            rapport.erreur = String((e && e.message) || e);
            postMessage(rapport);
        });
};
