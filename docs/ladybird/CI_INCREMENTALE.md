# CI Ladybird : iterations incrementales

La branche `feat/trigkey-interactive-latency` conserve le producteur canonique
`ladybird-native-browser` et son artefact `bouchaud-ladybird-native-browser`.
Les commandes Windows de telechargement et de fabrication USB ne changent pas.

## Gaspillages corriges

- Un cache vide appelait `build-vcpkg-gfx.sh` pour amorcer vcpkg : ce script
  compilait aussi l'ancien graphe M6 (dont Skia), avant le graphe du navigateur.
  Le bootstrap installe maintenant seulement le gestionnaire de paquets.
- Le triplet navigateur ne construit plus de bibliotheques Debug inutilisees :
  CMake construit deja le navigateur et ses generateurs en Release.
- Les dependances et la source sont sauvegardees des qu'elles sont pretes,
  avant la compilation C++. Une erreur C++ ne perd plus ces caches.
- Ninja et les sources preparees voyagent ensemble dans un cache incremental.
  Une preparation repart toujours d'upstream propre. Une synchronisation rsync
  par checksum preserve les dates des fichiers identiques, actualise les
  fichiers modifies et supprime ceux retires. Le depot source conserve son SHA
  upstream via un clone local partage. Aucun patch n'est saute.
- Les sept executables sont demandes ensemble a Ninja, qui peut ordonnancer
  leurs dependances communes et travaux independants.
- Ninja et ccache sont sauvegardes aussi apres un echec de compilation, apres
  l'etape de publication pour ne pas retarder la disponibilite de l'image.
- Les archives publiees utilisent une compression faible pour limiter le CPU
  consomme par leur emballage. Les verifications ELF et le smoke QEMU restent.

## Invalidation et limites

La cle Ninja couvre upstream, la toolchain, les scripts de configuration et le
triplet. Les modifications de sources et des scripts de preparation passent
par la comparaison de contenu, puis les dependances et commandes de Ninja.
Le cache vcpkg garde ses propres controles ABI; le stamp local couvre maintenant
le triplet. Les objets ccache sont une seconde possibilite de reutilisation en
cas de perte du cache Ninja. Les caches GitHub restent soumis aux limites de
stockage et a l'eviction : verifier leurs tailles et taux de restauration dans
les runs, surtout avec les autres workflows du depot. Une annulation brutale
peut encore empecher la sauvegarde; les checkpoints deja termines persistent.

Le premier run doit produire le nouveau cache Release et le cache Ninja.
Il peut encore etre long. Le gain principal concerne les runs suivants sur la
meme PR, avec une modification localisee de Ladybird. Aucun facteur x5/x10 ni
budget en minutes n'est mesure a ce stade. Comparer le temps des dependances,
de compilation et de sauvegarde entre le premier et le second run; consulter
les statistiques ccache et le nombre de commandes Ninja reellement executees.
Les caches de PR sont limites a leur reference GitHub; une nouvelle PR peut
avoir besoin d'un nouvel amorcage si main n'a pas de cache compatible.

## Verification locale

`python3 tools/ladybird/test-incremental-source.py` exerce Git et rsync reels :
source inchangee, changement de meme taille/meme date, ajout, suppression et
retrait d'un patch. La syntaxe shell, le YAML et les 92 garde-fous d'architecture
ont ete verifies. Le build complet et le smoke seront verifies par GitHub.

Reference du mode Release des ports :
https://learn.microsoft.com/en-us/vcpkg/users/triplets#vcpkg_build_type
