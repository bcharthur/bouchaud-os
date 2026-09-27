# P13.1 V5 — source BRDP cible-aware

La V4 a valide physiquement la configuration Windows suivante :

- ICS : `192.168.137.1/24`, UDP 67/68 ;
- LAB PC : `169.254.6.185/16`, `SkipAsSource=False` ;
- BRDP cible : `169.254.178.21:2222`.

Le dernier defaut etait dans le client PC : `BOUCHAUD_LAB_SOURCE_IP` lu depuis
`.env` etait applique a toute destination, y compris les faux serveurs unitaires
sur `127.0.0.1`. Sous Windows, une source `169.254.x.x` ne peut pas ouvrir une
connexion vers loopback, d'ou `WinError 10049` et deux tests faux-negatifs.

V5 rend le choix cible-aware :

- `--source-ip` explicite gagne toujours ;
- sinon `BOUCHAUD_LAB_SOURCE_IP`/.env n'est utilise automatiquement que pour
  une cible IPv4 link-local (`169.254.0.0/16`) ;
- loopback et les autres destinations laissent l'OS choisir la source.

Le harnais Windows V4 reste inchangé : `SkipAsSource=False` est la configuration
physiquement prouvee pour ARP + TCP/2222 tout en conservant ICS DHCP 67/68.
