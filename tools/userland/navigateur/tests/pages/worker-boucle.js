// Un worker qui ne rend jamais la main.
//
// C'est le cas que `terminate()` doit savoir traiter. Un arret qui attendrait
// poliment que le script veuille bien finir n'arreterait jamais celui-ci — et
// une page qui cree un worker hostile figerait le navigateur pour de bon.
//
// Il previent d'abord qu'il commence : sans cela, l'epreuve ne saurait pas si
// elle a interrompu un calcul ou un demarrage.

postMessage({ genre: "je-commence" });
while (true) { /* rien, et pour toujours */ }
