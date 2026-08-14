# Navigateur — boucle Qt et courtage asynchrones

Base du jalon : branche `Arthur`, commit
`24d36709c0c0395bca6202ec47299850b4ff8f22`.

## Ce qui change

- `QApplication::exec()` demarre avant la navigation initiale ;
- `VueRenderer.ouvre()` ne fait plus deux attentes synchrones pouvant atteindre
  60 secondes ;
- les evenements URL/TITLE/FRAME_READY remontent par le QTimer de 16 ms ;
- DNS/TCP/TLS/HTTP du courtier s'executent dans des workers du navigateur ;
- les workers ne touchent jamais au canal renderer ;
- le fil UI reinjecte au plus une reponse terminee par passage ;
- `transport.Courtier` serialise les transactions du protocole v1 afin que les
  fils de prechargement ne lisent pas la reponse les uns des autres ;
- `--verifie` attend maintenant le resultat asynchrone reel.

## Journal attendu

Au lancement :

    [bo] navigation initiale differee jusqu'a l'event-loop : bo:accueil

Puis la preuve principale que Qt vit :

    [bo] battement : 312 tics, ... trames, ... evenements recus

Pendant un chargement HTTP/HTTPS lent, le terminal, le WM et ces battements
continuent de vivre.

## Limites restantes

Le protocole v1 n'a pas encore de lecteur multiplexe par identifiant cote
renderer. Les requetes courtees y sont donc serialisees pour la correction. Le
prochain jalon de performance pourra ajouter un dispatcher de reponses par `id`
pour retrouver un vrai parallelisme des sous-ressources sans lecteurs
concurrents sur le meme flux.


## Contrats d'ouverture

- `commence_ouverture()` est non bloquant et reserve aux actions de l'UI Qt ;
- `ouvre()` conserve le contrat synchrone historique des epreuves et outils.

Le premier lancement attend egalement `READY` par battements successifs, sans bloquer l'event-loop.
