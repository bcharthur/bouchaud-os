// Un worker qui calcule vraiment, longtemps, sans jamais rendre la main.
//
// C'est l'epreuve qui distingue un vrai Worker d'une API qui en porte le nom.
// Tant que ce fichier tourne, il ne repond a rien : pas de minuterie, pas de
// message, pas de promesse. Si la page qui l'a lance continue de battre pendant
// ce temps, c'est que le calcul se fait ailleurs. Sinon, c'est qu'il se fait
// chez elle, et l'API ne sert a rien.
//
// Le calcul lui-meme n'a pas d'interet : il faut seulement qu'il ne puisse pas
// etre elimine par l'optimiseur et qu'il dure.

self.onmessage = function (evenement) {
    const message = evenement.data || {};
    if (message.genre !== "calcule") return;

    const depart = Date.now();
    let somme = 0;
    let tours = 0;
    // Une duree, pas un nombre d'iterations : la machine d'en face peut etre
    // dix fois plus lente que celle-ci, et l'epreuve doit durer pareil.
    const cible = Number(message.ms) || 200;
    while (Date.now() - depart < cible) {
        for (let i = 1; i <= 5000; i++) somme += Math.sqrt(i) / (i + 1);
        tours += 1;
    }
    postMessage({ genre: "fini", somme: somme, tours: tours,
                  duree: Date.now() - depart });
};
