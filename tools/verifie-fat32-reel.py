#!/usr/bin/env python3
"""L'ESP produite, donnee a des implementations FAT etrangeres.

`tools/fs/test_fat32.rs` relit ce qu'il ecrit avec un lecteur ecrit depuis la
specification. C'est deja mieux que de relire avec le code qui a ecrit -- mais
les deux lectures viennent de la meme personne, qui peut avoir mal compris la
meme chose deux fois.

Ce garde-fou donne l'image a `fsck.fat` et a `mtools`, qui n'ont jamais entendu
parler de ce projet, et qui refusent depuis des decennies ce que les
micrologiciels refusent. Un volume que `fsck.fat` accepte et dans lequel
`mdir` retrouve `kernel-x86_64` par son nom LONG est un volume dont on sait
qu'il ne sera pas la cause d'une machine qui ne demarre pas.

Sans ces outils, il PASSE en le disant. Faire echouer la barriere sur
l'outillage de la machine apprendrait a l'ignorer, ce qui coute plus cher que
ce qu'elle rapporte.
"""

import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
TEST = RACINE / "tools/fs/test_fat32.rs"

# Ce que le chargeur d'amorcage cherche sur l'ESP. Aucun de ces trois noms ne
# tient dans un nom 8.3 sans y perdre son identite.
ATTENDUS = ("kernel-x86_64", "boot.json", "ramdisk")


def somme_nom_court(nom):
    """La somme de controle des noms longs, reimplementee ici.

    Le test hote affirme des valeurs ; les recalculer depuis la specification,
    dans un autre langage, est ce qui evite qu'une valeur fausse soit gravee
    dans les deux endroits a la fois.
    """
    somme = 0
    for octet in nom:
        somme = (((somme & 1) << 7) + (somme >> 1) + octet) & 0xFF
    return somme


def controle_les_sommes(fautes):
    attendues = {
        b"THEQUI~1   ": 0xEE,
        b"KERNEL~1   ": 0x17,
        b"BOOT~1  JSO": 0xFE,
    }
    source = TEST.read_text(encoding="utf-8")
    for nom, valeur in attendues.items():
        if somme_nom_court(nom) != valeur:
            fautes.append(
                "la somme de %r vaut %#x et non %#x : la valeur gravee dans "
                "le test est fausse." % (nom, somme_nom_court(nom), valeur)
            )
        texte = nom.decode("ascii")
        if 'somme_nom_court(b"%s"), %#X' % (texte, valeur) not in source.upper().replace(
            "0X", "0x"
        ) and 'somme_nom_court(b"%s")' % texte not in source:
            fautes.append(
                "test_fat32.rs n'affirme plus la somme de %r." % nom
            )


def fabrique_image(dossier, fautes):
    """Compile le test hote et lui fait ecrire l'image."""
    binaire = dossier / "test_fat32"
    compilation = subprocess.run(
        ["rustc", "--edition", "2021", "--test", "-o", str(binaire), str(TEST)],
        capture_output=True,
        text=True,
    )
    if compilation.returncode != 0:
        fautes.append(
            "test_fat32.rs ne compile pas :\n%s" % compilation.stderr[:800]
        )
        return None
    image = dossier / "esp.img"
    execution = subprocess.run(
        [str(binaire), "--test-threads=1", "ecrit_l_image"],
        capture_output=True,
        text=True,
        env={**os.environ, "BO_FAT_IMAGE": str(image)},
    )
    if execution.returncode != 0 or not image.exists():
        fautes.append(
            "l'image n'a pas ete produite :\n%s" % (execution.stdout[-800:])
        )
        return None
    return image


def controle_fsck(image, fautes):
    """`fsck.fat` en lecture seule, avec le mode le plus strict."""
    # `-v` plutot que `-V` : le mode verbeux imprime la geometrie lue, ce qui
    # permet de verifier que l'outil comprend la MEME chose que nous -- taille
    # d'amas, nombre de FAT, amas de racine -- et pas seulement qu'il ne
    # trouve rien a redire.
    resultat = subprocess.run(
        ["fsck.fat", "-n", "-v", str(image)], capture_output=True, text=True
    )
    sortie = resultat.stdout + resultat.stderr
    if resultat.returncode != 0:
        fautes.append(
            "fsck.fat refuse le volume (code %d) :\n%s"
            % (resultat.returncode, sortie[:1500])
        )
        return
    # Un code de retour nul ne suffit pas : certaines incoherences sont
    # rapportees sans faire echouer l'outil.
    for signal in ("Dirty bit", "Free cluster summary wrong", "Cluster chain"):
        if signal.lower() in sortie.lower():
            fautes.append("fsck.fat signale « %s » :\n%s" % (signal, sortie[:800]))
    if "32 bit entries" not in sortie:
        fautes.append(
            "fsck.fat ne lit pas des entrees de FAT sur 32 bits ; le volume "
            "n'est donc pas du FAT32, et un micrologiciel n'y trouvera "
            "rien :\n%s" % sortie[:800]
        )
    if "Root directory start at cluster 2" not in sortie:
        fautes.append(
            "fsck.fat ne place pas la racine a l'amas deux :\n%s" % sortie[:800]
        )
    if "2 FATs" not in sortie:
        fautes.append(
            "fsck.fat ne voit pas deux copies de la FAT ; une seule suffit au "
            "systeme vivant et fait « reparer » le volume par un outil "
            "tiers :\n%s" % sortie[:800]
        )


def mdir(image, chemin, fautes):
    """Liste un repertoire avec mtools, sans fichier de configuration."""
    resultat = subprocess.run(
        ["mdir", "-i", str(image), chemin],
        capture_output=True,
        text=True,
        env={**os.environ, "MTOOLS_SKIP_CHECK": "1"},
    )
    if resultat.returncode != 0:
        fautes.append(
            "mdir refuse %s :\n%s" % (chemin, (resultat.stdout + resultat.stderr)[:800])
        )
        return ""
    return resultat.stdout


def controle_mtools(image, fautes):
    racine = mdir(image, "::/", fautes)
    for nom in ATTENDUS:
        if nom not in racine:
            fautes.append(
                "mtools ne retrouve pas « %s » par son nom LONG. Ecrit en 8.3, "
                "ce fichier devient introuvable pour le chargeur d'amorcage, "
                "et la machine ne demarre pas sans un mot d'explication.\n%s"
                % (nom, racine[:600])
            )
    boot = mdir(image, "::/EFI/BOOT", fautes)
    if "BOOTX64" not in boot.upper():
        fautes.append(
            "mtools ne retrouve pas BOOTX64.EFI dans /EFI/BOOT :\n%s" % boot[:600]
        )
    # La taille annoncee doit etre la vraie taille, pas un multiple d'amas.
    if not re.search(r"\b40\s*000\b|\b40000\b", racine + boot):
        fautes.append(
            "la taille de BOOTX64.EFI n'est pas celle qu'on a ecrite ; une "
            "taille arrondie a l'amas fait lire des octets de remplissage "
            "comme du code.\n%s" % boot[:600]
        )


def controle_file(image, fautes):
    """Un second avis, d'un outil qui ne fait que LIRE l'en-tete.

    `file` et `fsck.fat` ne partagent pas leur code. Les deux doivent conclure
    « FAT 32 bits » : un seul des deux le disant signalerait un en-tete
    ambigu, c'est-a-dire un en-tete que certains micrologiciels liront
    autrement.
    """
    if shutil.which("file") is None:
        return
    resultat = subprocess.run(["file", str(image)], capture_output=True, text=True)
    sortie = resultat.stdout
    if "FAT (32 bit)" not in sortie:
        fautes.append("`file` ne voit pas du FAT 32 bits :\n%s" % sortie[:400])
    if "BOUCHAUDESP" not in sortie:
        fautes.append("`file` ne lit pas l'etiquette du volume :\n%s" % sortie[:400])


def main():
    fautes = []
    controle_les_sommes(fautes)

    manquants = [outil for outil in ("fsck.fat", "mdir") if shutil.which(outil) is None]
    if manquants:
        print(
            "fat32 reel : %s absent(s), verification externe non executee "
            "(les sommes de controle, elles, ont ete recalculees)"
            % ", ".join(manquants)
        )
        if fautes:
            for faute in fautes:
                print("  - %s" % faute)
            return 1
        return 0

    if shutil.which("rustc") is None:
        print("fat32 reel : rustc absent, verification externe non executee")
        return 0

    with tempfile.TemporaryDirectory() as brut:
        dossier = Path(brut)
        image = fabrique_image(dossier, fautes)
        if image is not None:
            controle_fsck(image, fautes)
            controle_mtools(image, fautes)
            controle_file(image, fautes)

    if fautes:
        print("fat32 reel : %d probleme(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print(
        "fat32 reel : fsck.fat accepte le volume, mtools y retrouve "
        "kernel-x86_64, boot.json et ramdisk par leurs noms longs"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
