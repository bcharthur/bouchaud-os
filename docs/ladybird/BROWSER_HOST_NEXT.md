# Browser Host Ladybird — consolidation

Cette version consolide les patches Browser Host précédents. Le point important
est que `prepare-full-browser-host.py` conserve désormais **en même temps** :

- le takeover IPC upstream de WebContent (`SOCKET_TAKEOVER`) ;
- le resource root `/usr/share/ladybird` sous `BOUCHAUD_PORT` ;
- les répertoires XDG/runtime et `--disable-sql-database` de la première phase ;
- le vrai Compositor activé en BrowserHost ;
- le raccord M11 à la page créée par `WebView::Application` ;
- le chemin explicite `/usr/libexec/ladybird/<helper>` ;
- la console JavaScript série également disponible en BrowserHost ;
- RequestServer, ImageDecoder et WebWorker lancés par les mécanismes upstream.

Les anciens scénarios M8/M9 gardent leurs chemins historiques pour les
régressions. Le BrowserHost utilise les IPC synchrones du vrai hôte et ne doit
pas retomber sur les pots locaux WebContent.

La validation CI dédiée doit prouver au minimum Canvas 2D + WebWorker avant
d'interpréter ce port comme stable.
