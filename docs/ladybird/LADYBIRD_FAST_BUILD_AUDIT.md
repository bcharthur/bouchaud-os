# Audit du workflow Ladybird-fast-build

Le workflow restaure séparément source, cache binaire vcpkg, build Ninja et
ccache. Une modification de `BouchaudChrome.h` ne change pas la clé vcpkg, mais
la clé du build contient `github.sha`; le `restore-keys` récupère donc le dernier
build du même SHA upstream avant une recompilation incrémentale.

L'étape `Dependances navigateur` reconstruit volontairement
`third_party/vcpkg-browser-installed` à chaque run. Elle doit normalement
restaurer les paquets depuis `~/.cache/vcpkg/archives`, mais cet install root
n'est lui-même ni restauré ni sauvegardé. Le script convertit aussi le clone
vcpkg en historique complet et effectue des fetch réseau. Une durée de deux
heures à cette étape indique donc un cache binaire absent/évincé ou non sauvegardé,
pas une recompilation nécessaire causée par le header M11.

Aucune modification automatique du cache n'est faite dans ce correctif runtime:
mettre l'install root en cache sans le rendre dépendant du manifeste et des
overlays risquerait de réintroduire les dépendances obsolètes que le nettoyage
actuel élimine. Le prochain chantier CI doit publier un cache binaire vcpkg
immuable indexé par baseline + manifeste transformé + triplet, afficher
`cache-hit` et `ccache --show-stats`, puis mesurer séparément vcpkg, compilation
et édition de liens.
