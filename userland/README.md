# Bouchaud userland

Ce dossier définit la destination des composants natifs ring 3 de Bouchaud OS.

Architecture prévue :

- `libs/bouchaud-sdk` : API système native et wrappers IPC ;
- `libs/bouchaud-graphics` : surfaces, scène et contrats graphiques ;
- `libs/bouchaud-ui` : toolkit UI déclaratif, avec adaptateur Dioxus envisagé ;
- `services/compositor` : compositeur en espace utilisateur ;
- `services/init` : démarrage et supervision des services ;
- `services/network` : services réseau déplaçables hors noyau ;
- `apps` : applications first-party.

Le GUI et la chaîne navigateur existants ne sont pas déplacés brutalement : ils
migreront composant par composant lorsque les contrats noyau/userland seront
stables.
