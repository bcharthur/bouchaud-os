// Un WebSocket ouvert depuis un Worker.
//
// Le code de `WebSocket` est litteralement le meme que celui de la fenetre : il
// vient de `prelude_partage.js`, et seul l'hote qui repond aux operations
// change. C'est ce que cette epreuve verifie — pas que `WebSocket` existe, mais
// qu'il fonctionne avec un autre hote sans avoir ete recrit.

self.onmessage = function (evenement) {
    const message = evenement.data || {};
    if (message.genre !== "va") return;
    const rapport = { genre: "rapport", recus: [] };

    const connexion = new WebSocket(message.url);
    connexion.onopen = function () {
        rapport.ouvert = true;
        rapport.protocole = connexion.protocol;
        connexion.send("bonjour depuis le worker");
    };
    connexion.onmessage = function (e) {
        rapport.recus.push(e.data);
        connexion.close(1000, "fini");
    };
    connexion.onclose = function (e) {
        rapport.code = e.code;
        rapport.propre = e.wasClean;
        rapport.fait = true;
        postMessage(rapport);
    };
    connexion.onerror = function (e) {
        rapport.erreur = String(e.message || "erreur");
    };
};
