# Performance Ladybird

Le noyau émet des timestamps monotones en nanosecondes pour le clic/autostart,
chaque début d'exec et la première trame composée. Chaque marqueur donne aussi
`since_click_ms`, permettant de mesurer click→exec et click→first paint sans
dépendre des ticks PIT.

Les valeurs doivent être collectées sur la même accélération QEMU, le même
nombre de CPU et le même disque. Aucun chiffre TCG ne doit être présenté comme
WHPX. Les marqueurs READY propres aux services, DOM_READY, première requête et
INTERACTIVE doivent être ajoutés aux programmes userland concernés: le noyau ne
doit pas assimiler `exec` à «service prêt».

Le compteur file-backed distingue déjà les faults zero/file. La prochaine étape
est de compter octets/commandes/durée ATA et d'ajouter un readahead ELF borné;
aucun gain startup n'est revendiqué avant cette mesure runtime.
