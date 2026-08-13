// Le worker le plus simple qui prouve quelque chose.
//
// Ce qu'il verifie tient en deux choses :
//
//   * il repond a ce qu'on lui envoie, donc les messages traversent dans les
//     deux sens ;
//   * il dit ce qu'il voit de son monde, donc on peut verifier que ce monde
//     n'est pas celui d'un document. Un worker qui aurait `document` serait un
//     worker qui partage l'arbre avec le fil de l'interface — exactement ce
//     qu'il ne doit pas etre.

const surface = {
    aDocument: typeof document !== "undefined",
    aWindow: typeof window !== "undefined",
    aHtmlElement: typeof HTMLElement !== "undefined",
    aLocalStorage: typeof localStorage !== "undefined",
    aAlert: typeof alert !== "undefined",
    selfEstGlobal: self === globalThis,
    aPostMessage: typeof postMessage === "function",
    aFetch: typeof fetch === "function",
    aSetTimeout: typeof setTimeout === "function",
    aSetInterval: typeof setInterval === "function",
    aQueueMicrotask: typeof queueMicrotask === "function",
    aUrl: typeof URL === "function",
    aUrlSearchParams: typeof URLSearchParams === "function",
    aTextEncoder: typeof TextEncoder === "function",
    aTextDecoder: typeof TextDecoder === "function",
    aIndexedDb: typeof indexedDB === "object" && indexedDB !== null,
    aWebSocket: typeof WebSocket === "function",
    portee: typeof WorkerGlobalScope === "function",
};

self.onmessage = function (evenement) {
    const message = evenement.data;
    if (message && message.genre === "surface") {
        postMessage({ genre: "surface", surface: surface });
        return;
    }
    if (message && message.genre === "echo") {
        postMessage({ genre: "echo", recu: message.charge });
        return;
    }
    if (message && message.genre === "minuterie") {
        // Les minuteries battent sur le fil du worker, pas sur celui de la
        // page : si elles n'existaient pas, cette reponse n'arriverait jamais.
        let tours = 0;
        const jeton = setInterval(function () {
            tours += 1;
            if (tours >= 3) {
                clearInterval(jeton);
                postMessage({ genre: "minuterie", tours: tours });
            }
        }, 1);
        return;
    }
    if (message && message.genre === "texte") {
        const octets = new TextEncoder().encode(message.charge);
        postMessage({ genre: "texte",
                      octets: Array.from(octets),
                      relu: new TextDecoder().decode(octets) });
        return;
    }
    if (message && message.genre === "url") {
        const adresse = new URL(message.charge, self.location.href);
        postMessage({ genre: "url", href: adresse.href,
                      origine: adresse.origin, chemin: adresse.pathname });
        return;
    }
    if (message && message.genre === "microtache") {
        const ordre = [];
        ordre.push("sync");
        queueMicrotask(function () {
            ordre.push("microtache");
            postMessage({ genre: "microtache", ordre: ordre });
        });
        ordre.push("sync-fin");
        return;
    }
    postMessage({ genre: "inconnu" });
};
