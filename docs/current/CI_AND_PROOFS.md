# CI et preuves

Niveaux :
STATIC → HOST → QEMU-TCG → QEMU-KVM → PHYSICAL.

Workflows :
CI Fast, Reliability V3, Integration, os-primitives,
ladybird-native-browser.

TCG juge la stabilité et mesure la performance.
KVM juge stabilité + performance.

Le projet a déjà rencontré un faux vert d'horloge dû à une mesure manquante ;
les tests actuels exigent chaque mesure intermédiaire.
