// Temoin execute par `js`, l'interpreteur d'upstream.
//
// Ce fichier n'existe pas pour couvrir le langage — Test262 est fait pour cela.
// Il existe pour qu'un programme **d'upstream**, compile sans modification,
// execute du JavaScript sur Bouchaud OS et le dise.
//
// Chaque ligne vise donc une couche du portage plutot qu'une regle du langage,
// et une seule brique defaillante suffit a la faire echouer.

let echecs = 0;

function verifie(quoi, obtenu, attendu) {
    const ok = String(obtenu) === String(attendu);
    if (!ok) ++echecs;
    console.log(`${ok ? "  ok    " : "  ECHEC "} ${quoi} = ${obtenu}${ok ? "" : ` (attendu ${attendu})`}`);
}

console.log("== temoin js (interpreteur d'upstream) ==");

// Le jalon M4 : une ligne imprimee par LibJS depuis Bouchaud OS. Elle etait
// jusqu'ici produite par notre propre sonde ; la voici produite par
// l'interpreteur d'upstream, ce qui est la meme preuve en plus fort.
console.log("Hello Bouchaud");

// Le coeur du langage : analyse, code-octet, interpreteur assembleur genere
// par flapc.
verifie("1 + 2", 1 + 2, 3);
verifie("fermeture et recursion", (function f(n) { return n <= 1 ? 1 : n * f(n - 1); })(10), 3628800);

// Classes, iterateurs, destructuration : la syntaxe moderne complete.
class Compte {
    #total = 0;
    ajoute(...valeurs) { for (const v of valeurs) this.#total += v; return this; }
    get total() { return this.#total; }
}
verifie("classe et champ prive", new Compte().ajoute(1, 2, 3, 4).total, 10);

const [premier, ...reste] = [10, 20, 30];
verifie("destructuration", `${premier}/${reste.join(",")}`, "10/20,30");

// BigInt -> LibCrypto et libtommath.
verifie("BigInt 2n ** 128n", (2n ** 128n).toString(), "340282366920938463463374607431768211456");

// Unicode -> LibUnicode et les donnees ICU.
verifie("normalize NFC", "é".normalize("NFC").codePointAt(0).toString(16), "e9");
verifie("segmentation d'un drapeau", [..."\u{1F1EB}\u{1F1F7}"].length, 2);

// Intl -> les donnees CLDR.
verifie("Intl.NumberFormat", new Intl.NumberFormat("de-DE").format(1234.5), "1.234,5");
verifie("Intl.DateTimeFormat", new Intl.DateTimeFormat("fr-FR", { timeZone: "UTC" }).format(new Date(0)), "01/01/1970");

// JSON -> simdjson.
verifie("JSON", JSON.stringify(JSON.parse('{"a":[1,2,{"b":null}]}')), '{"a":[1,2,{"b":null}]}');

// Expressions regulieres -> LibRegex et libregex_rust.
verifie("regex nommee", "2026-08-15".replace(/(?<a>\d+)-(?<m>\d+)-(?<j>\d+)/, "$<j>/$<m>/$<a>"), "15/08/2026");

// Exceptions JS, implementees sans exceptions C++ (-fno-exceptions partout).
verifie("try/catch", (() => { try { null.x; } catch (e) { return e.constructor.name; } })(), "TypeError");

// Promesses et microtaches : la file de la VM.
//
// `queueMicrotask` n'existe **pas** ici, et c'est correct : c'est une API du
// Web (HTML), fournie par LibWeb, pas par LibJS. Un moteur JavaScript nu n'a que
// les promesses de la norme ECMAScript. Le premier jet de ce fichier l'appelait
// et `js` a repondu « ReferenceError: 'queueMicrotask' is not defined » — une
// bonne reponse a une mauvaise attente, et un rappel utile de la frontiere
// exacte entre ce qui est construit et ce qui ne l'est pas encore.
let ordre = [];
Promise.resolve().then(() => ordre.push("micro"));
ordre.push("sync");
Promise.resolve().then(() => {
    ordre.push("fin");
    verifie("ordre des microtaches", ordre.join(","), "sync,micro,fin");
    console.log(`RESULTAT : ${echecs} verification(s) en echec`);
});
