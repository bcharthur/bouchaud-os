# Arborescence attendue par les applications Linux

## Etat verifie

`src/kernel/sysroot.rs` cree deja au demarrage :

    /etc
    /proc  /proc/self  /proc/sys/kernel  /proc/sys/vm
    /sys/class/graphics/fb0  /sys/class/input
    /sys/devices/system/cpu  /sys/devices/system/cpu/cpu0
    /usr/share/fonts/truetype/dejavu   (DejaVu Sans, Bold, Mono)

`src/kernel/fd.rs` expose comme peripheriques synthetiques :

    /dev/null  /dev/zero  /dev/random  /dev/urandom
    /dev/tty  /dev/console  /dev/stdin  /dev/stdout  /dev/stderr
    /dev/tty0  /dev/tty1  /dev/tty2  /dev/vc/0
    /dev/fb0  /dev/fb  /dev/graphics/fb0
    /dev/input/event0  /dev/input/event1
    /dev/dsp  /dev/dsp0  /dev/audio  /dev/sound/dsp

L'archive userland est deployee dans le RAMFS au demarrage (121 fichiers,
28 repertoires).

## Ce qui manque

| Chemin | Usage | Priorite |
|---|---|---|
| `/lib64/ld-linux-x86-64.so.2` | binaires **glibc** dynamiques | apres musl |
| `/proc/self/exe` | chemin du binaire — tres utilise | **elevee** |
| `/proc/self/maps` | allocateurs, GC balayeurs de pile | **elevee** |
| `/proc/self/cmdline`, `/proc/self/environ` | introspection | moyenne |
| `/proc/cpuinfo`, `/proc/meminfo` | dimensionnement des pools | moyenne |
| `/etc/resolv.conf`, `/etc/hosts` | resolution | elevee si voie curl |
| `/etc/ssl/certs` | autorites racine | elevee si voie OpenSSL |
| `/tmp` inscriptible | quasi tout | **elevee** |
| `/home/<user>`, `$HOME` | profils | present |
| `/dev/shm` | `shm_open` | moyenne |
| `/dev/ptmx`, `/dev/pts` | terminaux | faible |

`/proc/self/maps` merite d'etre souligne : c'est par la que plusieurs
ramasse-miettes — dont celui que LibJS embarque via LibGC — determinent les
bornes de pile. Un contenu absent ou faux se paie en objets vivants recoltes,
c'est-a-dire en corruption differee.

## Principe

On ne cree pas une arborescence Linux « pour faire vrai ». Chaque entree est
ajoutee parce qu'une **trace** montre une application qui la demande. Le traceur
de `MASTER_PLAN.md` alimente cette table.
