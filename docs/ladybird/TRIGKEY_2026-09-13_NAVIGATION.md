# Navigation sur Trigkey : diagnostic du test de 23:16

Source : blackbox-extract-20260913-232108.zip, serial.log, memory.log,
samples.log et fatal.log. Image declaree : SHA256
77E14F8C2E064CEE878CF1D4909E3AD1D06ECD57B04B252FE92C7B8F687CCE10.
Le retour utilisateur valide la fluidite clavier/souris sur cette image.

## Observations et limites

- 23:16:52 : lancement du navigateur avec DNS 10.0.2.3,
  verdict sans-configuration, lien=0. RequestServer confirme ce DNS deux fois.
- 23:16:59 : DHCP fournit 192.168.1.97, passerelle et DNS 192.168.1.254.
  L'environnement du processus deja lance ne peut pas suivre ce changement.
- 23:17:34 : WebContent utilise 99 % d'un coeur; le WM rapporte encore
  0 trames et 0 octet recu du navigateur. Les FPS du bureau ne mesurent pas
  le debit de rendu de Ladybird.
- 23:17:36-41 : acces aux informations CPU, aux polices, puis creation d'un
  thread supplementaire WebContent. Le processus progresse, mais tres lentement.
- fatal.log est vide; aucun panic noyau ni evenement OOM dans les traces lues.
  Cela ne prouve pas l'absence de tous les defauts du navigateur.
- Les traces actuelles ne permettent pas d'attribuer les dizaines de secondes
  de demarrage a une fonction precise. Polices, initialisation JS et cout
  avant main restent a mesurer. Il serait incorrect d'annoncer ce point corrige.

## Corrections de ce lot

1. Le noyau publie resolv.conf lors de l'initialisation reseau, de l'obtention
   du bail et de la perte de lien. Sans configuration, il ne publie plus le
   DNS QEMU comme s'il etait utilisable sur le Trigkey.
2. RequestServer relit ce fichier avant chaque requete HTTP(S) du BrowserHost.
   Une modification du DNS utilise set_dns_server et reset_connection upstream.
   Un reseau non configure rend une erreur immediate, sans attente du faux DNS.
   Une requete deja en vol n'est pas rejouee automatiquement; recharger apres
   retour du reseau. La resolution des WebSockets conserve son chemin upstream.
3. Accueil local sans JavaScript ni ressource distante, avec formulaire de
   recherche Google. Il ne lance plus Google automatiquement avant DHCP.
4. Delai de connexion TCP/TLS de 90 a 15 secondes pour Bouchaud. Ce n'est pas
   un delai global de chargement; la phase DNS et les transferts ont leur cycle.
5. Connexions curl bornees a 6 par hote et 16 au total par client RequestServer,
   pour contenir le cout des connexions simultanees. Pas de baisse de validation
   des certificats TLS ni de suppression des controles ELF.
6. Jalons de demarrage, attachement GUI et premiere trame sur stderr, visible
   dans la serie, pour separer initialisation des polices, VM JS et pont GUI.
7. Script local WSL suivi dans tools/reference/build-ladybird-local.ps1 :
   Ubuntu 26.04, normalisation CRLF, caches conserves, copie finale PowerShell
   apres verification des ELF. Pas de telechargement obligatoire depuis la CI.

## Verification

Le parseur DNS C++ a ete execute avec AddressSanitizer/UBSan (verification des
fuites desactivee car l'environnement ne permet pas son inspection de /proc).
Cas : DNS absent, ancien DNS, nouveau bail, deconnexion, IPv4 invalide, CRLF.
Le patch s'applique deux fois sans duplication aux fichiers upstream epingles.
Syntaxe Python et Bash validees; garde-fou Stage 2 valide.
91/92 gardes d'architecture passent : le dernier refuse le binaire noyau
precedent car les sources ont change. Rust n'est pas disponible dans cette
session; compilation noyau, compilation complete Ladybird et boot restent a
valider par CI et sur la machine. Aucun gain de temps physique n'est encore
mesure pour ce lot.

## Prochain test physique

1. Ouvrir le navigateur avant la fin du DHCP : accueil local utilisable.
2. Apres le bail, rechercher un mot ou ouvrir https://example.com/.
3. Attendre BROWSER_DNS_UPDATED server=192.168.1.254 et M11_FIRST_FRAME.
4. Deconnecter/reconnecter Ethernet, puis recharger : nouvelle resolution.
5. Capturer les paires BROWSER_STARTUP_BEGIN/END pour chaque phase. Leur absence
   avant le premier jalon orientera l'analyse vers le chargement/pre-main.
6. Verifier recherche, liens, retour, rechargement, plusieurs onglets, molette
   et saisie; exporter une nouvelle blackbox sans confondre FPS WM et navigateur.
