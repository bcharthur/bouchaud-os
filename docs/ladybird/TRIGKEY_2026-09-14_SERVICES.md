# Trigkey : services, entrees et fin de session

Capture examinee : blackbox-extract-20260914-002442.zip, session 00:19:51–00:20:03.
Le journal fatal est vide. Le dernier extrait serie montre seulement le debut
du bureau, sans lancement Ladybird ni marque de fin. Il ne permet donc pas de
mesurer un crash du moteur, la latence de frappe dans une page ou le temps
complet d'extinction. Le reseau passe lien UP a 00:19:56 mais reste
sans-configuration a 00:20:00. Le clavier utilise le secours GET_REPORT;
la souris produit des rapports acceptes. Le prechargement speculatif remplit
64 Mio pour un seul fichier en 187 ms. Aucun manque de dalle n'est releve.

## Corrections

- Session Ladybird unique lancee apres le premier rendu du bureau, minimisee,
  avec ses vrais canaux GUI et ses enfants natifs. Ouvrir Ladybird restaure
  cette session. Pas de relance automatique en boucle apres un crash.
- Application Services sur le bureau et dans le menu : demarrage/arret de
  l'arbre, PID des moteurs vivants et etat reseau. Les moteurs crees a la
  demande restent signales comme tels. Presence d'un PID n'est pas readiness.
  CPU et somme des RSS reutilisent les mesures existantes toutes les cinq secondes;
  les pages partagees peuvent etre comptees dans plusieurs RSS.
- La session en arriere-plan passe en priorite normale; le foyer du navigateur
  retrouve la classe interactive. Aucun changement des garanties SMP/BKL.
- Suppression du scan speculatif de 64 Mio en concurrence avec les services sur
  Stage2. Les pages sont chargees a la demande par les vrais processus.
- FIFO clavier bornee a 256 transitions, conservees en cas de canal plein;
  reprise lors du pompage. Une saturation totale signale l'erreur et ferme la
  session plutot que laisser une touche coincee. Test hote de la FIFO.
- Configure transmis lors d'un changement de foyer, meme sans redimensionnement.
- Le filtre de disponibilite DNS ne bloque plus les ressources hors HTTP(S).
  Migration des arbres prepares V1 et application repetee sans doublons.
- Retrait de la pause preboot de deux secondes et des affichages hardware.
  Logo des le prechargeur, chargeur sans INFO a l'ecran; progression par jalons dans le
  noyau. Pas de delai artificiel pour une animation. Les fautes restent visibles.
- Ecran d'arret anime entre les lots de journaux. Vidage borne vers un instantane
  fini, budget de cinq secondes entre transferts, reprises en cas de contention USB,
  retour reel du SYNCHRONIZE CACHE. Un dernier vidage conserve le resultat
  de la synchronisation des fichiers. Un transfert USB deja engage reste soumis
  a son propre timeout.
  Le succes visible exige vidage, marque, synchronisation et persistance reussis.
  Une sauvegarde incomplete est signalee; le systeme ne retient pas indefiniment
  une extinction sur un support absent ou defaillant.

## Validation et limites

Les contrats de preparation Ladybird et l'application repetee du patch reseau
sont testes sur des fixtures issues des sources. Les controles d'architecture
restent obligatoires, adaptes au nouveau choix explicite de demarrage graphique.
CI Fast compile maintenant aussi la cible UEFI reference-desktop; ses tests hote
incluent l'ordre appui/relachement sous contre-pression et la saturation FIFO.

CI Fast 34787586962 est verte sur ef15066a : compilation legacy et UEFI,
Clippy, 93 garde-fous et tests hote dont la nouvelle FIFO. Les finitions de
mesures et de preboot sont soumises a la meme barriere, avec compilation du shim.
Cette machine de travail ne dispose pas de WSL ni du Trigkey : le C++ complet
reste a compiler localement et les comportements physiques restent a mesurer. Pas de promesse de navigateur parfaitement fonctionnel ni de
ressources parfaitement utilisees sans nouvelle mesure physique.

Test attendu : bureau sans terminal automatique, Services affiche les PID,
ouvrir Ladybird reutilise le PID hote; Ctrl+L puis recherche apres DHCP, saisie
continue et touches maintenues, changement de foyer, arret/demarrage des services,
puis Eteindre et extraction de la session portant FIN. Mesurer lancement ->
premiere trame, DNS, CPU/RSS par processus et delai d'arret dans le nouveau ZIP.
