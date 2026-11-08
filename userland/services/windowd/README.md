# windowd — extraction progressive du gestionnaire de fenêtres en ring 3

`windowd` est un véritable **exécutable natif isolé**, compilé par
`userland/libs/bouchaud-sdk/build-program.sh`, lancé par
`bo_process_spawn` et piloté par **un canal IPC atténué**. Sa politique
géométrique se trouve dans `window_policy.h` et n'utilise **aucun** accès
au noyau, au framebuffer, à Ladybird ou à DOOM.

## Ce qui fonctionne dans cette tranche

- Identifiants de fenêtres indépendants des applications, maximum 16.
- Ouverture, déplacement, redimensionnement, focus, fermeture et interrogation.
- Clipping géométrique sur le bureau 1280×720, refus de tailles invalides.
- Autorité déterminée par le handle de démarrage : le client ne fournit pas
  d'identité de sécurité dans sa requête.
- Refus de manipuler les fenêtres appartenant à un autre principal.
- Un processus parent natif démarre `windowd`, échange les requêtes et
  vérifie qu'il termine proprement, **sans faire mourir le parent**.

## Preuve exécutable

```bash
bash tools/ci/run_native_services.sh \
  target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin 4
```

Puis idem SMP1. Le test exige les marqueurs :

```text
WINDOWD_RING3_READY
WINDOWD_RING3_IPC_OK
NATIVE_SERVICES_OK
```

Les tests hôte découvrent `tools/native/test_windowd_policy.cpp`.
Un service simplement compilé ne satisfait **pas** cette preuve.

## Important : ce que ce chantier ne prouve PAS encore

**Atelier utilise encore son gestionnaire de fenêtres en ring 0.**
Le prototype ne remplace pas `src/gui/window_manager.rs`, ne possède
pas le framebuffer, ne reçoit pas encore les entrées HID de production et
ne gère pas les fenêtres réellement visibles dans Atelier. Le compositeur
existant `userland/services/composited` possède déjà une preuve de
surface/IPC, mais il n'est pas encore relié à `windowd` et au bureau réel.

Pour le faire devenir le gestionnaire de fenêtres **par défaut** :
1. Brancher un serveur d'affichage ring 3 qui détient la capacité de
   présentation/scanout et reçoit les événements via un service d'entrées.
2. Relier `windowd` à `composited` : ID de fenêtre, PID, surface accordée,
   z-order, clip de dégâts, focus, buffers libérés.
3. Construire un `appd` ring 3 à partir des manifestes ; séparer le
   processus de l'application (Ladybird, DOOM...) du gestionnaire de fenêtre.
4. Déporter le menu, dock, thèmes et les applications
   `src/gui/apps/{terminal,file_explorer,calculator,notes,rustpad,...}`
   dans l'espace utilisateur. Faire tourner ces applications **comme
   processus ring 3**, pas seulement leurs algorithmes.
5. Comparer les captures/QEMU au bureau existant et prouver qu'une
   application qui plante ne plante ni le WM ni le noyau.
6. En dernier, remplacer le lancement du desktop ring 0 par `windowd` /
   `composited` et enlever les anciens chemins. Ne jamais le faire avant
   l'équivalent fonctionnel et la qualification P13/Trikey.

## Frontière de sécurité

Le wire v1 est un échange entre `windowd` et **un seul canal déjà autorisé**
par le superviseur (principal=1). Il ne constitue pas un protocole multi-client
public. La variante multi-clients devra utiliser **un handle par principal**
avec table de capacités, ACL et vérification des surfaces. Ne jamais permettre
au client de choisir librement un PID ou le propriétaire de la fenêtre.

Le service de démonstration n'a accès ni à la carte graphique ni au disque :
aucune fausse promesse de compositeur de production.
