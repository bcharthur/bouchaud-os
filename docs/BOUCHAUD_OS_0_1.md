# Bouchaud OS 0.1 — Scope et Definition of Done

## Intention

Bouchaud OS 0.1 est une première version **x86_64, QEMU-first**, reproductible
et utilisable pour un workload desktop/navigateur défini. Ce n'est ni le support
PC universel ni la fin de la vision. Du code présent n'est DONE qu'avec les
artefacts runtime exigés ci-dessous.

## MUST HAVE

### Plateforme et noyau

- x86_64 sous une configuration QEMU versionnée ; boot SMP4 répétable ;
- ELF ring 3, VM, mémoire partagée et filesystem stables sous le workload ;
- scheduler SMP, signaux, timers, futex/waits et syscalls nécessaires aux apps ;
- TLB shootdown correct sous stress VM/SMP ;
- aucune panic, deadlock ou stall BKL catastrophique ; hold BKL maximal :
  **threshold TBD after baseline**.

### Desktop et applications

- input, fenêtres, focus, déplacement, minimisation/maximisation, menu, taskbar
  et curseur utilisables ;
- composition event-driven et damage corrects sans corruption après stress ;
- idle CPU/wakeups : **threshold TBD after baseline** ;
- Terminal utilisable, Rustpad capable d'ouvrir/modifier/sauver, Fichiers capable
  de parcourir/manipuler le stockage persistant ;
- Calculatrice livrée, mais non bloquante si le packaging final doit être réduit.

### Navigateur et réseau

- e1000 QEMU, IPv4 (DHCP ou statique documenté), TCP, DNS et HTTPS stables ;
- WebContent/Ladybird avec adresse, navigation, historique, clic, clavier et
  scroll nécessaires au scénario ;
- pages de référence courtes et contrôlées lorsque possible ;
- erreurs réseau visibles et récupération sans reboot.

« Navigateur utilisable » ne signifie pas Web universel, frontend upstream
complet, sandbox complète, multi-onglet ou GPU.

### Robustesse et documentation

- build/run vérifiés depuis un checkout propre par la procédure documentée ;
- logs série, outils, configuration QEMU et verdicts archivés ;
- documentation build, architecture, statut, limitations et dépannage à jour ;
- modèle de sécurité honnête, sans revendiquer une sandbox absente.

## NICE TO HAVE

- validation sur **une** machine x86_64 choisie et documentée ;
- meilleurs seuils latence/idle après baseline ;
- davantage de pages, onglets, raccourcis et redimensionnement navigateur ;
- pilotes strictement utiles au matériel choisi ;
- prototype de window server userland sans bloquer le chemin 0.1 validé.

## NOT IN 0.1

- tous les PC/laptops : ACPI complet, NVMe/AHCI généralisé, xHCI, USB HID,
  Wi-Fi, HDA, batterie, suspend/resume et tous GPU ;
- AArch64, QEMU `virt` ou Raspberry Pi comme exigence ; Bouchaud One final ;
- exécution PE/Windows — parseur et `PreparedImage` ne suffisent pas ;
- frontend Ladybird complet, tout le Web, sandbox complète, GPU ou capabilities
  achevées ;
- microkernel pur ou migration forcée de toute politique avant la release.

## Workload de référence

1. boot QEMU x86_64 SMP4 et vérification des quatre CPU actifs ;
2. démarrage desktop et WebContent/Ladybird ;
3. navigation HTTPS par nom vers une fixture et `https://example.com/` ;
4. adresse, lien, scroll, clic, clavier, fenêtres, focus, menu et curseur répétés ;
5. Terminal, Rustpad, Fichiers ; écriture, reboot, lecture d'un témoin persistant ;
6. charge VM/processus/IPC/réseau en parallèle ;
7. période idle puis reprise ;
8. collecte panics/stalls, BKL, compositions, damage, overflows, mémoire, réseau.

## Definition of Done

Ces nombres sont des gates pragmatiques, pas des seuils scientifiques. Chaque
case exige un log/rapport attaché au checkpoint 0.1.

### Reproductibilité

- [ ] 10 boots SMP4 consécutifs réussissent avec la même image/configuration.
- [ ] Chaque boot annonce quatre CPU actifs et atteint le desktop.
- [ ] Le build réussit depuis un checkout propre dans l'environnement versionné.
- [ ] Une seconde personne ou CI reproduit build, image et workload depuis la doc.

### Kernel / robustesse

- [ ] Aucun panic, deadlock, double fault ou watchdog fatal pendant la campagne.
- [ ] Le workload complet tient 30 minutes (gate initiale, pas preuve illimitée).
- [ ] Stress VM/processus/IPC valide mappings, shared memory, futex, signaux et
  shootdown sans corruption détectée.
- [ ] Hold BKL sous **threshold TBD after baseline**, provenance/distribution
  publiées, sans stall catastrophique.
- [ ] Latences scheduler/input sous **threshold TBD after baseline**.

### GUI

- [ ] Curseur/fenêtres/focus/menu/taskbar ne laissent aucune corruption visible.
- [ ] Aucun overflow damage dans le workload ; fallback testé si capacité atteinte.
- [ ] Rendu damage-clipped comparé au rendu complet sur les transitions de référence.
- [ ] Sortie d'idle sans freeze ni événement perdu.
- [ ] Idle compositor/CPU/wakeups sous **threshold TBD after baseline**.

### Applications, stockage et réseau

- [ ] Terminal exécute les commandes 0.1 avec sorties/erreurs correctes.
- [ ] Rustpad ouvre, édite, sauve et rouvre un fichier UTF-8.
- [ ] Fichiers parcourt, crée, renomme et supprime dans le périmètre documenté.
- [ ] Un fichier témoin persiste après reboot, contenu/hash vérifié.
- [ ] DHCP ou statique donne une connectivité répétable.
- [ ] DNS/TCP survivent aux répétitions/timeouts sans reboot.
- [ ] HTTPS valide chaîne et nom sur fixture/site public ; limites documentées.

### Navigateur et release

- [ ] WebContent/Ladybird tient 30 minutes sans panic ni freeze desktop.
- [ ] Fixture et `example.com` chargent par nom et acceptent les interactions.
- [ ] Google reste exploratoire jusqu'à définition d'un scénario stable ; aucune
  réussite n'est présumée aujourd'hui.
- [ ] Après erreur DNS/TCP/TLS, une navigation suivante réussit sans reboot.
- [ ] `STATUS.md`, limitations et sécurité reflètent le checkpoint final.
- [ ] Commit, configuration, logs et métriques sont archivés ; aucune case n'est
  cochée sur la compilation seule.
