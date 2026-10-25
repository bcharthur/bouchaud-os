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

La version conservee quitte ce contexte IRQ SEULEMENT apres abandon du
peuplement, avant les diagnostics de la faute fatale ring 3. IF est masque
pour cette transition, avec profondeur exactement 1 exigee. La continuation
reste celle de la tache ; une vraie IRQ qui l'interrompt conserve son
enter/exit. Le peuplement, les fautes resolues et les autres handlers gardent
leur contexte historique. Aucune garde de preemption ni taille d'anneau
n'est relachee. IRQ/panique/IF masque gardent le repli borne.

Enfin, le destructeur masque IF avant de rendre GS utilisateur. Sinon une
IRQ ring 0 entre SWAPGS et IRET utilisait le GS utilisateur. IRET restaure
le IF de la trame. Cette fenetre est prouvee par l'ordre des instructions ;
son implication dans les anciennes corruptions C5 n'est pas demontree.

Le candidat plus large abd7f807 demotait des l'entree et ajoutait un point
sur avant le retour utilisateur. Ses preuves `trappe-*` : BIOS et
UEFI compiles, contexte SMP1/SMP4, 24 fautes multi-fils (#UD/#DE/#GP/#PF),
C6 avec 9064 lectures, 11375 changements d'affinite, aucun recul ni
depassement ; 7 tests IRQ et 21 mutations du garde de preemption. Les
quatre replays console passent, 600/600 chacun et zero perte ; le quatrieme
execute secours=1, relogees=1 en 20 ms. Le controle OFF garde son repli.

La CI [40187123850](https://github.com/bcharthur/bouchaud-os/actions/runs/40187123850)
confirme ces quatre replays, zero perte et relogement en 20 ms. Trois
repetitions locales OFF passent aussi. Mais la campagne KVM
[40187123841](https://github.com/bcharthur/bouchaud-os/actions/runs/40187123841)
reste NON_CONVAINCANT ; C5 est reproduit dans le scenario 48 fautes puis
persist-course, avec et sans la correction dmesg. Retirer seulement le
point sur ne suffit pas (1 rouge / 6). L'ouverture generale du peuplement
et le point sur sont donc retires par prudence, sans leur attribuer une
cause non prouvee. La version fatale seule reproduit aussi C5 (1 rouge / 6),
signature historique printf_core+0, CR2 nul alors que l'instruction lue est
un push. Cela ne prouve ni un defaut QEMU ni sa cause noyau.

La version etroite a ses propres preuves `fatale-*`, sept tests IRQ et
22 mutations. Les essais refuses sont conserves. Le banc reproductible
`tools/ci/run_faute_persistance.sh` verifie aussi 64 lignes coherentes du
journal et garde un verdict rouge si une seule repetition echoue.
