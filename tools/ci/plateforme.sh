#!/usr/bin/env bash
#
# La PLATEFORME DE REFERENCE de Bouchaud OS, en un seul endroit.
#
# # Pourquoi ce fichier existe
#
# Huit scripts de CI lancent QEMU, et chacun ecrivait ses propres options. Il
# n'y avait donc pas de plateforme de reference : il y en avait huit, et rien
# ne disait laquelle comptait. Changer de machine demandait de modifier huit
# fichiers en esperant n'en oublier aucun -- et un oubli ne se voit pas, il
# donne simplement un test qui continue de passer sur l'ancienne.
#
# # La decision du chantier 10
#
# La plateforme de reference est x86_64 + QEMU **q35**, pas i440fx.
#
# Ce n'est pas une preference : i440fx est un chipset de 1996, et tout ce qui
# est moderne -- NVMe, MSI-X, PCIe, AHCI -- suppose q35. Continuer a tout
# tester sur i440fx revient a ne jamais rencontrer les cas que le materiel
# reel produit : sur q35 les peripheriques sont derriere des PONTS RACINE
# PCIe, donc sur d'autres bus que le zero.
#
# C'est exactement ce que l'enumeration PCI ne voyait pas avant ce chantier.
#
# # La migration est progressive, et le repli est nomme
#
# `BOUCHAUD_MACHINE` choisit la machine. Le defaut reste `pc` -- i440fx --
# jusqu'a ce que le boot q35 soit demontre : basculer huit campagnes d'un coup
# sans pouvoir les executer ici serait un pari, pas une migration.
#
#   BOUCHAUD_MACHINE=q35 tools/ci/run_qemu_smoke.sh bootimage.bin
#
# Le jour ou q35 boote, ce defaut change ICI, une seule fois, et les huit
# campagnes suivent.

# La machine de reference, et celle utilisee par defaut aujourd'hui.
BOUCHAUD_MACHINE_REFERENCE=${BOUCHAUD_MACHINE_REFERENCE:-q35}
BOUCHAUD_MACHINE=${BOUCHAUD_MACHINE:-pc}

# Les options de machine, deduites du profil choisi.
#
# `q35` amene un pont hote PCIe et des ports racine : c'est ce qui rend la
# topologie a plusieurs bus reelle, et donc testable.
bouchaud_machine_args() {
    case "$BOUCHAUD_MACHINE" in
        q35)
            printf '%s' "-machine q35"
            ;;
        pc|i440fx|"")
            printf '%s' "-machine pc"
            ;;
        *)
            echo "profil de machine inconnu : $BOUCHAUD_MACHINE" >&2
            return 1
            ;;
    esac
}

# Un disque NVMe, quand le profil le permet.
#
# NVMe n'existe pas sur i440fx : le demander y ferait echouer QEMU au lancement,
# ce qui se lirait comme une regression du noyau alors que c'est la machine qui
# ne sait pas.
bouchaud_nvme_args() {
    local image=$1
    case "$BOUCHAUD_MACHINE" in
        q35) printf '%s' "-drive format=raw,file=$image,if=none,id=nvm0 -device nvme,serial=bouchaud0,drive=nvm0" ;;
        *) printf '%s' "" ;;
    esac
}

bouchaud_profil_resume() {
    echo "plateforme: machine=$BOUCHAUD_MACHINE (reference=$BOUCHAUD_MACHINE_REFERENCE)"
}
