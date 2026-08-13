# Bouchaud OS V0.13 — GUI interactif et trajectoire haute resolution

## Objectifs livres

- Fenetre **Terminal** dans le GUI : saisie clavier non bloquante, sortie capturee, reutilisation du shell existant.
- **Lanceur d'applications** : Terminal, Systeme, Notes et Navigateur HTTP prepare.
- **Apps maison** : fenetres natives de statut systeme, notes/roadmap et navigateur texte placeholder.
- **Sortie GUI rapide** : `Echap`, `exit` ou `logout` rendent la main au mode texte.

## Haute resolution / bootloader 0.11

Le bureau reste compatible avec le mode VGA 13h actuel pour conserver une image bootable avec `cargo bootimage`.
La migration haute resolution ciblee est la suivante :

1. remplacer la dependance historique `bootloader 0.9`/`bootimage` par la chaine `bootloader 0.11` + API framebuffer ;
2. passer `kernel_main` sur les informations de framebuffer fournies par le bootloader ;
3. ajouter un backend `drivers::gfx` framebuffer lineaire RGB/BGR en plus du backend VGA 13h ;
4. conserver les primitives GUI actuelles (`clear`, `fill_rect`, `rect`, `draw_text`, `present`) pour que les apps ne changent pas ;
5. monter progressivement en 640x480, 800x600 puis resolution native QEMU.

Cette PR garde donc une architecture GUI prete a migrer, sans casser le boot actuel.

## Navigateur texte HTTP

L'app `Navigateur HTTP` est volontairement un placeholder graphique tant que le driver e1000 et la pile TCP ne sont pas disponibles.
Activation prevue apres :

1. e1000 PCI ;
2. Ethernet + ARP + IPv4 ;
3. ICMP de verification ;
4. UDP/DNS ;
5. TCP minimal ;
6. HTTP GET texte.
