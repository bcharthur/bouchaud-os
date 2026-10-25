# Faute de page utilisateur : continuation et retour protege

Le replay CI `40186341131` du timer e4216531 reproduit la saturation :
controle OFF, 600/600 lignes utilisateur, 39 differees vidangees et 1260
perdues ; le verrou 3 s perd aussi 7 lignes. La sonde provoque des #PF
utilisateur pour produire ses diagnostics noyau.

Cause structurelle dans le code : GsGuard comptait la faute synchrone comme
une IRQ active pendant tout son traitement, alors que cette faute rouvre
IF, attend des pages et peut ceder sa pile. `cession_volontaire_sure`
refusait donc la contre-pression de la console (irq_depth != 0). Le chemin
Occupe alimentait les petits anneaux d'urgence, sans attente possible.

La faute venue de ring 3 quitte maintenant ce contexte IRQ apres capture
de CR2, avant STI, avec profondeur exactement 1 exigee. La continuation
reste celle de la tache ; une vraie IRQ qui l'interrompt conserve son
enter/exit. Les fautes noyau et les autres handlers ne sont pas demotes.
Le point sur existant est appele apres resolution, hors verrous, avant
la frontiere de comptabilite utilisateur. Aucune garde de preemption ni
taille d'anneau n'est relachee. IRQ/panique/IF masque gardent le repli borne.

Enfin, le destructeur masque IF avant de rendre GS utilisateur. Sinon une
IRQ ring 0 entre SWAPGS et IRET utilisait le GS utilisateur. IRET restaure
le IF de la trame. Cette fenetre est prouvee par l'ordre des instructions ;
son implication dans les anciennes corruptions C5 n'est pas demontree.

Preuves locales, dans `target/p13-architecture-20261025/trappe-*` : BIOS et
UEFI compiles, contexte SMP1/SMP4, 24 fautes multi-fils (#UD/#DE/#GP/#PF),
C6 avec 9064 lectures, 11375 changements d'affinite, aucun recul ni
depassement ; 7 tests IRQ et 21 mutations du garde de preemption. Les
quatre replays console passent, 600/600 chacun et zero perte ; le quatrieme
execute secours=1, relogees=1 en 20 ms. Le controle OFF garde son repli.

Ces essais ne prouvent pas P2/P4 KVM. Le correctif attend une campagne sur
son propre SHA ; les echecs du timer e4216531 sont conserves. Les repetitions
OFF et les A/B suivants sont consignes dans le document de reprise.
