# Trigkey : bilan mesure et ameliorations du 13 septembre 2026

Base consolidee : `f2398ef558115b6a8897a8de5da704362041ffb4`.
Branche de travail : `feat/trigkey-interactive-latency`.

## Ce que prouve le dernier essai

Source : `blackbox-extract-20260913-201829.zip`, session du 13 septembre
a 20:07:12. Aucun commit ou ELF exact n'est joint a cette archive : les
adresses ne sont pas symbolisees contre un noyau different.

| Observation | Mesure | Portee |
|---|---:|---|
| Entree UEFI / framebuffer | 1920 x 1080, BGR | Demarrage physique etabli |
| CPU logiques en ligne et timers actifs | 16 / 16 | SMP physique etabli, pas un benchmark d'equilibrage |
| Controleurs xHCI actifs | 2 | Deux claviers et une souris enumeres |
| Echantillons blackbox | 30 | Fenetre echantillonnee de 14,524 s ; dernier point a 20,476 s depuis boot |
| Scrutations HID | 249,93 / s | Delta de compteurs entre premier et dernier echantillons |
| Evenements / rapports Interrupt-IN | 0 / 0 | Aucun transport d'entree normal observe |
| Rapports clavier / souris | 1227 / 0 | Clavier via GET_REPORT ; 1227 n'est PAS un nombre de frappes |
| Rearmements / coups de sonnette | 3 / 84 | Les relances de sonnette n'ont pas produit de rapport |
| Reseau | Lien Gigabit puis configuration IPv4 obtenue | Pas de preuve HTTP/HTTPS dans ce run |
| Erreurs d'ecriture blackbox | 0 | Aucun enregistrement fatal ; ne prouve pas l'endurance |
| Navigateur | Aucun handshake M11 dans le journal | FPS et latence du navigateur non mesures |

Les 15 paires `gfx-enter/gfx-exit` sont des operations de presentation,
pas une serie temporelle de FPS du navigateur. Le journal s'arrete apres une
vingtaine de secondes : il ne prouve pas que tout le test utilisateur a ete
enregistre. Il ne permet pas d'affirmer pourquoi la souris n'a jamais parle.

Reproduction des compteurs, sans extraire le ZIP :

```powershell
python .\tools\reference\analyse-fluidite-blackbox.py "chemin\blackbox-extract-20260913-201829.zip"
```

## Avancement des douze chantiers

Un pourcentage global d'OS fini serait arbitraire. Les mesures de code et les
preuves d'execution n'ont pas la meme portee.

| Chantier | Etat utile aujourd'hui | Ce qui manque pour l'objectif Trigkey |
|---|---|---|
| 1. Concurrence / BKL | 81 routes syscall hors BKL sur 159 ; 78 restent sous BKL | Finaliser les domaines et mesurer la contention sous navigateur |
| 2. Scheduler / preemption | 16 CPU et timers verifies dans le ZIP | Mesure wake-to-run et endurance en charge interactive |
| 3. Memoire | Allocateur par pages, dalles, caches ; pas de saturation observee ici | Pression memoire, fragmentation et defauts de page du navigateur |
| 4. Graphique | Framebuffer GOP en ecriture combinee, damage, compositeur noyau ; tranche ring 3 presente | GPU Vega accelere et migration complete du compositeur non etablis |
| 5. Stockage | NVMe lu sur machine, USB pour blackbox ; GPT et commit A/B dans le code | Installation/persistance et resistance aux coupures sur cible |
| 6. Securite | W^X, canaris, droits et sandbox dans le code et les tests | Validation d'ensemble, durcissement et separation du chrome navigateur |
| 7. ABI / IPC | Processus ring 3, canaux GUI et ABI native | Couverture et stabilisation produit ; compatibilite ne signifie pas dependance a Linux |
| 8. Ladybird | Portage, chrome, onglets et services presents | Premiere navigation et interactivite non prouvees par CE run |
| 9. Reseau | Pilote Ethernet et configuration physique observes | DNS/HTTPS repetes, debits et latences sur cible |
| 10. Materiel | UEFI, GOP, SMP, Ethernet, NVMe et enumeration USB passent plusieurs etapes physiques | Reception souris fiable, hotplug, veille, audio/Wi-Fi selon peripheriques |
| 11. Fiabilite | Suites hote, garde-fous et campagnes QEMU automatisees | Endurance physique et correlation fiable image/ELF/logs |
| 12. Ergonomie | Bureau, fenetres, texte, navigation et reglages dans le code | Comportement complet et fluide verifie de bout en bout |

Le projet est un OS experimental qui demarre sur la cible, avec une base
fonctionnelle importante. Le dernier test ne valide pas encore un poste de
navigation utilisable au quotidien. Aucun des douze chantiers n'est declare
termine par cette analyse.

## Changements de cette branche

1. **USB : un seul TD en vol par endpoint.** Un nouvel armement ne peut plus
   dupliquer un transfert deja en attente, notamment pendant le hotplug.
   Les achevements anciens sont ignores au lieu de consommer le nouveau TD.
2. **Reprise USB : conserver la position et le cycle producteur.** Plus de
   remise a zero de l'anneau qui rendrait d'anciens TRB a nouveau valides.
   Lever le halt USB avant la reprise du contexte xHC. Examiner aussi les
   endpoints muets depuis le demarrage. Ne pas deduire un anneau vide du
   seul contexte de sortie Running, dont le dequeue peut etre ancien.
3. **Diagnostic USB et capacites.** Instantane initial par endpoint apres le
   delai de grace ; correction des champs Hi/Lo HCSPARAMS2 des scratchpads.
   Le decodage est couvert par des valeurs asymetriques. Cette erreur
   d'allocation n'est pas presentee comme la cause prouvee du silence HID.
4. **Bureau evenementiel.** Retirer l'echeance artificielle a 2 ms quand le
   fil HID existe. Garder la scrutation de secours si ce fil ne demarre pas,
   y compris lorsque le bureau n'a aucune autre echeance.
5. **Navigateur non bloquant.** File bornee de 16 messages GUI, conservation
   des offsets d'ecriture et reprise au prochain tick sur EAGAIN/EINTR.
   Plus de boucle de 64 `usleep(1000)` dans l'envoi. Si la file est pleine,
   une invalidation complete reste en attente : un degat n'est pas oublie.
   Une fermeture definitive du canal reste une erreur, pas une attente infinie.
6. **Rendu navigateur.** Plafond porte de 30 a 60 images/s pour la page
   initiale et les nouveaux onglets. C'est un plafond, pas un resultat mesure.
   Les pages inchangees restent gerees par l'invalidation du moteur.
7. **Mesures.** `wm_tours`, `wm_entrees`, `wm_trames` ajoutes aux echantillons
   blackbox ; analyseur compatible avec les anciennes archives. Les moyennes
   de trames incluent les periodes inactives, elles ne sont pas un maximum FPS.

Reference du decodage xHCI : [specification Intel, sections 5.3.4 et 4.6.8](https://www.intel.com/content/dam/www/public/us/en/documents/technical-specifications/extensible-host-controler-interface-usb-xhci.pdf).

## Verification

- 92 garde-fous d'architecture ; le controle du sommeil HID suit maintenant
  la fonction de politique testee au lieu de l'ancien bloc inline.
- 70 suites Rust hote, six suites C++ dont la file GUI, tests Python de
  fiabilite. Quatre tests de symbolisation sautes avant construction d'un ELF.
- Compilation complete pour `targets/x86_64-bouchaud_os_uefi.json`, options
  `uefi-boot,reference-bringup,reference-desktop` ; verification ELF ET_DYN.
- La compilation complete Ladybird et les scenarios physiques restent des
  validations distinctes : un test C++ de la file ne compile pas LibWeb entier.
- Aucune mesure apres patch sur le Trigkey disponible pendant cette passe.

## Essai et reconstruction

Le noyau seul ne met pas a jour WebContent. `-ForceLadybird` reconstruit le
ramdisk depuis les binaires presents ; il ne recompile pas Ladybird.
Il faut d'abord recuperer l'artefact du workflow `ladybird-native-browser`
construit pour cette branche/PR, puis reconstruire l'image USB.

```powershell
git fetch origin --prune
git switch feat/trigkey-interactive-latency
git pull --ff-only
# Remplacer 123456789 par le run de cette branche qui a produit l'artefact.
.\run.ps1 -Ladybird -RefreshLadybird -LadybirdRunId 123456789
# Fermer QEMU, puis reconstruire l'image physique :
powershell -ExecutionPolicy Bypass -File .\tools\reference\IMAGE-TRIGKEY.ps1 -ForceLadybird
```

Le marqueur `M11_INTERACTIVE_TRANSPORT_READY fps_cap=60 output=nonblocking`
identifie le navigateur reconstruit. Sans lui, ne pas attribuer une mesure
de navigateur aux modifications de cette branche.

Tester successivement : 30 s au repos, frappe/repetition/AltGr, mouvement,
clic/glissement, debranchement/rebranchement, page locale, DNS/HTTPS, defilement,
redimensionnement et plusieurs onglets. Conserver la nouvelle blackbox et le
manifeste/ELF de l'image. Attendus : rapports souris et Interrupt-IN qui
progressent, `wm_entrees` puis `wm_trames` en mouvement, absence de faute et
absence de progression des erreurs d'ecriture. Une souris immobile peut
legitimement ne produire aucun rapport. Ne pas utiliser ce critere au repos.

## Branches

La consolidation conserve les deux parents dans `main`. Les quatre anciennes
branches sont integrees. Leur suppression distante n'etait pas disponible via
la connexion utilisee ; la commande suivante verifie a nouveau leur ascendance
et refuse une suppression si leur SHA distant a change entretemps :

```powershell
powershell -ExecutionPolicy Bypass -File .\tools\reference\nettoie-branches-integrees.ps1
```

La branche d'amelioration reste distincte de `main` pour la validation physique.
