#!/usr/bin/env python3
"""Ce que l'installation sur le disque interne ne doit jamais redevenir.

Installer est irreversible. Les regles ci-dessous portent donc autant sur ce
que l'installateur ECRIT que sur ce qu'il refuse d'ecrire -- et sur la chaine
de construction, parce qu'un installateur correct qui ne trouve pas sa charge
utile produit exactement le meme resultat qu'un installateur faux : une machine
qui ne demarre pas.

La derniere regle donne l'ESP telle qu'une VRAIE installation la produit a
`fsck.fat` et a `mtools`. Sans ces outils, elle passe en le disant.
"""

import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
INSTALL = RACINE / "src/platform/pc/installation.rs"
DISPOSITION = RACINE / "src/platform/pc/installation/disposition.rs"
PERSISTANCE = RACINE / "src/fs/persistance.rs"
COMMANDES = RACINE / "src/shell/commands.rs"
STAGE2 = RACINE / "src/platform/pc/stage2.rs"
BUREAU = RACINE / "src/gui/window_manager.rs"
BUILDER = RACINE / "tools/reference/uefi-image-builder/src/main.rs"
STAGE_PS = RACINE / "tools/reference/stage-install-payload.ps1"
PREPARE_PS = RACINE / "tools/reference/prepare-reference-ladybird.ps1"
TEST = RACINE / "tools/platform/test_installation.rs"

# Les trois fichiers sans lesquels une ESP est un systeme de fichiers valide
# que le micrologiciel ignore.
OBLIGATOIRES = ("bootx64.efi", "kernel-x86_64", "boot.json")


def sans_commentaires(source):
    sortie = []
    dans_chaine = False
    i = 0
    while i < len(source):
        c = source[i]
        if dans_chaine:
            if c == "\\":
                sortie.append("  ")
                i += 2
                continue
            if c == '"':
                dans_chaine = False
            sortie.append(c)
            i += 1
            continue
        if c == '"':
            dans_chaine = True
            sortie.append(c)
            i += 1
            continue
        if c == "/" and i + 1 < len(source) and source[i + 1] == "/":
            while i < len(source) and source[i] != "\n":
                sortie.append(" ")
                i += 1
            continue
        if c == "#" and source[i - 1 : i] != '"':
            pass
        sortie.append(c)
        i += 1
    return "".join(sortie)


def corps(source, signature):
    debut = 0
    while True:
        trouve = source.find(signature, debut)
        if trouve < 0:
            return None
        ouvrante = -1
        declaration = -1
        profondeur = 0
        i = trouve
        while i < len(source):
            c = source[i]
            if c in "([<":
                profondeur += 1
            elif c in ")]>":
                if profondeur > 0:
                    profondeur -= 1
            elif profondeur == 0:
                if c == "{":
                    ouvrante = i
                    break
                if c == ";":
                    declaration = i
                    break
            i += 1
        if ouvrante >= 0:
            break
        if declaration < 0:
            return None
        debut = declaration + 1
    profondeur = 0
    i = ouvrante
    while i < len(source):
        if source[i] == "{":
            profondeur += 1
        elif source[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[ouvrante : i + 1]
        i += 1
    return None


def regle_zone_de_persistance(disposition, persistance, fautes):
    """La partition systeme est dimensionnee par la zone qu'elle doit porter.

    `disposition.rs` est inclus tel quel dans un test hote qui n'a pas le
    noyau : il ne peut pas importer la constante, il la recopie. Deux nombres
    qui divergent donneraient une partition creee, formatee, declaree
    installee, et sur laquelle chaque `sync` echoue en silence.
    """
    ici = re.search(r"SECTEURS_PERSISTANCE:\s*u64\s*=\s*([0-9_]+)", disposition)
    la = re.search(r"SECTEURS_ZONE:\s*u64\s*=\s*([0-9_]+)", persistance)
    if ici is None:
        fautes.append("disposition.rs : SECTEURS_PERSISTANCE a disparu.")
        return
    if la is None:
        fautes.append("persistance.rs : SECTEURS_ZONE a disparu.")
        return
    if int(ici.group(1).replace("_", "")) != int(la.group(1).replace("_", "")):
        fautes.append(
            "disposition.rs annonce %s secteurs de persistance, persistance.rs "
            "en reserve %s. Une partition systeme dimensionnee sur le mauvais "
            "nombre est creee, formatee, declaree installee -- et chaque "
            "« sync » y echoue en silence."
            % (ici.group(1), la.group(1))
        )
    bloc = corps(disposition, "pub fn plan(")
    if bloc is None:
        fautes.append("disposition.rs : plan() introuvable.")
    elif "SECTEURS_PERSISTANCE" not in bloc:
        fautes.append(
            "disposition.rs : plan() ne borne plus la partition systeme par la "
            "zone de persistance."
        )


def regle_esp_plafonnee(disposition, fautes):
    """L'ESP ne mange jamais le disque.

    Une charge annoncee absurde -- un compte qui deborde, une archive dont la
    longueur est un nombre au hasard -- produit sinon un plan arithmetiquement
    valide ou la partition systeme est un residu.
    """
    bloc = corps(disposition, "pub fn plan(")
    if bloc is None:
        return
    if "zone / 4" not in bloc.replace(" ", " "):
        fautes.append(
            "disposition.rs : le plafond de taille de l'ESP a disparu ; une "
            "charge absurde donnerait un disque sans place pour les donnees."
        )
    if "ESP_MINIMUM_MIO" not in bloc:
        fautes.append(
            "disposition.rs : le plancher de taille de l'ESP a disparu ; sous "
            "65 525 amas un micrologiciel conforme lit du FAT16."
        )
    if "aligne(" not in bloc:
        fautes.append(
            "disposition.rs : les partitions ne sont plus alignees ; un SSD "
            "desaligne fait des lecture-modification-ecriture invisibles."
        )


def regle_rien_sans_demande(commandes, fautes):
    """La commande ne fait que REGARDER par defaut.

    Un systeme live qui partitionnerait le disque interne detruirait la machine
    sur laquelle on voulait juste l'essayer, et ce n'est pas rattrapable.
    """
    bloc = corps(commandes, "pub fn installer(")
    if bloc is None:
        fautes.append("commands.rs : la commande `installer` a disparu.")
        return
    if "if !demande" not in bloc:
        fautes.append(
            "commands.rs : `installer` n'exige plus une demande explicite ; il "
            "ecrirait sur le disque interne sans qu'on le lui demande."
        )
    appel = bloc.find("installation::installe(")
    garde = bloc.find("if !demande")
    if appel < 0:
        fautes.append("commands.rs : `installer` n'appelle plus l'installation.")
    elif garde < 0 or appel < garde:
        fautes.append(
            "commands.rs : l'installation est lancee AVANT le controle de la "
            "demande explicite."
        )


def regle_disque_occupe(install, fautes):
    """Un disque qui porte autre chose est refuse, sauf demande explicite.

    Un utilisateur qui a un autre systeme sur ce disque doit se voir refuser
    l'installation avec une phrase, pas la decouvrir detruite.
    """
    bloc = corps(install, "pub fn installe(")
    if bloc is None:
        fautes.append("installation.rs : installe() introuvable.")
        return
    if "if !ecrase" not in bloc or "DisqueOccupe" not in bloc:
        fautes.append(
            "installation.rs : un disque portant des partitions etrangeres "
            "n'est plus refuse ; elles seraient detruites sans avertissement."
        )
    # Le refus doit precede la premiere ecriture.
    refus = bloc.find("DisqueOccupe")
    ecriture = bloc.find("ecrit_table")
    if refus >= 0 and ecriture >= 0 and refus > ecriture:
        fautes.append(
            "installation.rs : le controle d'occupation vient APRES l'ecriture "
            "de la table ; le disque est deja detruit quand on le refuse."
        )


def regle_liste_fermee(install, fautes):
    """Seuls des noms connus atterrissent dans l'ESP.

    Ce qui atterrit dans une partition d'amorcage decide de ce que le
    micrologiciel demarre. Une liste ouverte ferait de ce chemin un moyen
    d'ecrire un fichier arbitraire dans la partition d'amorcage.
    """
    bloc = corps(install, "fn nom_statique(")
    if bloc is None:
        fautes.append("installation.rs : la liste fermee des noms a disparu.")
        return
    if "_ => None" not in bloc:
        fautes.append(
            "installation.rs : la liste des noms deposes dans l'ESP n'est plus "
            "fermee."
        )
    for nom in OBLIGATOIRES:
        cible = "BOOTX64.EFI" if nom == "bootx64.efi" else nom
        if cible not in bloc:
            fautes.append("installation.rs : « %s » n'est plus depose." % cible)


def regle_charge_obligatoire(install, fautes):
    """Les trois fichiers d'amorcage sont obligatoires.

    Sans le chargeur, l'ESP posee est un systeme de fichiers valide que le
    micrologiciel ignore : une installation qui se declare reussie et une
    machine qui ne demarre pas.
    """
    bloc = corps(install, "fn charge_d_amorcage(")
    if bloc is None:
        fautes.append("installation.rs : charge_d_amorcage() introuvable.")
        return
    for nom in OBLIGATOIRES:
        if nom not in bloc:
            fautes.append(
                "installation.rs : « %s » n'est plus exige de l'archive." % nom
            )
    if "ChargeIntrouvable" not in bloc:
        fautes.append(
            "installation.rs : une charge absente n'arrete plus l'installation."
        )


def regle_fenetre_avant_enregistrement(install, fautes):
    """La fenetre est posee AVANT que le volume soit enregistre.

    Un volume enregistre sur une fenetre encore vide rend un descripteur absent
    au premier appelant qui regarde, et la persistance conclut qu'il n'y a pas
    de disque.
    """
    bloc = corps(install, "pub fn monte_le_systeme_installe(")
    if bloc is None:
        fautes.append("installation.rs : monte_le_systeme_installe() introuvable.")
        return
    pose = bloc.find("SYSTEME.pose(")
    enregistre = bloc.find("enregistre(Volume::DONNEES")
    if pose < 0 or enregistre < 0:
        fautes.append(
            "installation.rs : la partition systeme n'est plus exposee comme "
            "volume de donnees."
        )
    elif pose > enregistre:
        fautes.append(
            "installation.rs : le volume est enregistre AVANT que la fenetre "
            "soit posee ; la persistance conclura qu'il n'y a pas de disque."
        )


def regle_fenetre_bornee(install, fautes):
    """Une requete hors fenetre est refusee.

    Ce n'est pas une erreur theorique : c'est le mecanisme meme par lequel un
    systeme de fichiers ecrit sur ses voisins. La persistance occupe la FIN de
    son volume -- non bornee, elle ecrirait dans les derniers blocs du DISQUE,
    c'est-a-dire par-dessus la table GPT de secours.
    """
    bloc = corps(install, "fn adresse(&self, lba: u64, n: usize)")
    if bloc is None:
        fautes.append("installation.rs : la traduction d'adresse de la fenetre a disparu.")
        return
    if "checked_add" not in bloc or "> blocs" not in bloc:
        fautes.append(
            "installation.rs : la fenetre ne borne plus ses requetes ; la "
            "persistance ecrirait par-dessus la table GPT de secours."
        )


def regle_chaine_de_construction(builder, stage_ps, prepare_ps, fautes):
    """L'archive porte vraiment ce que l'installateur y cherche.

    Un installateur correct qui ne trouve pas sa charge produit le meme
    resultat qu'un installateur faux.
    """
    if "--charge-installation" not in builder:
        fautes.append(
            "uefi-image-builder : le mode d'emission de la charge "
            "d'installation a disparu."
        )
    bloc = corps(builder, "fn emet_charge_installation(")
    if bloc is None:
        fautes.append("uefi-image-builder : emet_charge_installation() introuvable.")
    else:
        for nom in OBLIGATOIRES:
            if nom not in bloc:
                fautes.append(
                    "uefi-image-builder : « %s » n'est plus emis dans la "
                    "charge d'installation." % nom
                )
        if "configuration(" not in bloc:
            fautes.append(
                "uefi-image-builder : la charge d'installation ne partage plus "
                "la configuration d'amorcage de l'image ; la machine installee "
                "demarrerait avec des reglages que personne n'a choisis."
            )
    if "--charge-installation" not in stage_ps:
        fautes.append(
            "stage-install-payload.ps1 : la charge n'est plus emise."
        )
    for nom in OBLIGATOIRES:
        if nom not in stage_ps:
            fautes.append(
                "stage-install-payload.ps1 : « %s » n'est plus exige." % nom
            )
    if "stage-install-payload.ps1" not in prepare_ps:
        fautes.append(
            "prepare-reference-ladybird.ps1 : la charge d'installation n'est "
            "plus deposee dans l'archive ; « installer » ne trouvera rien."
        )
    # Elle doit etre deposee AVANT que l'archive soit fabriquee.
    depot = prepare_ps.find("stage-install-payload.ps1")
    archive = prepare_ps.find("$Make $Scenario")
    if depot >= 0 and archive >= 0 and depot > archive:
        fautes.append(
            "prepare-reference-ladybird.ps1 : la charge est deposee APRES la "
            "fabrication de l'archive ; elle n'y sera pas."
        )


def regle_montage_differe(stage2, install, bureau, fautes):
    """Une machine installee monte sa persistance -- APRES le bureau.

    La regle a change de forme, pas d'objectif. Elle exigeait que l'amorcage
    monte lui-meme le systeme installe ; c'est ce qui faisait dependre le
    premier affichage d'un disque dont on ne sait rien. Un NVMe qui n'acheve
    pas ses commandes n'echouait pas, il faisait ATTENDRE, interruptions
    masquees, une commande apres l'autre -- et l'utilisateur voyait un ecran
    fige, sans clavier ni souris, avant tout bureau.

    Ce qui reste exige : la persistance EST montee. Elle l'est simplement
    depuis le bureau, une fois la premiere image rendue. Les trois maillons
    sont verifies, parce qu'en casser un seul rendrait la machine RAM-only
    apres installation sans qu'aucun test ne le dise.
    """
    if "monte_le_systeme_installe()" in stage2:
        fautes.append(
            "stage2.rs : le systeme installe est de nouveau monte PENDANT "
            "l'amorcage ; un disque muet y bloque le premier affichage."
        )
    if "differe_le_montage()" not in stage2:
        fautes.append(
            "stage2.rs : le montage du systeme installe n'est plus demande ; "
            "la machine resterait en RAM-only apres l'installation."
        )

    differe = corps(install, "pub fn execute_le_montage_differe(")
    if differe is None:
        fautes.append(
            "installation.rs : execute_le_montage_differe() a disparu ; plus "
            "rien ne monte le systeme installe."
        )
    else:
        if "monte_le_systeme_installe()" not in differe:
            fautes.append(
                "installation.rs : le montage differe ne monte plus le systeme "
                "installe."
            )
        if "persistance::monte()" not in differe:
            fautes.append(
                "installation.rs : le montage differe ne restaure plus la "
                "persistance."
            )

    # Le bureau DECLENCHE le montage ; il ne l'execute plus lui-meme.
    #
    # `lance_le_montage_differe` cree un fil noyau et rend la main. C'est la
    # meme propriete -- « demande, donc fait » -- obtenue sans que le
    # compositeur porte le travail sur sa pile et dans son quantum. Exiger
    # l'appel EN LIGNE reviendrait a interdire cette separation.
    if "lance_le_montage_differe()" not in bureau:
        fautes.append(
            "window_manager.rs : le bureau ne declenche plus le montage differe ; "
            "il serait demande et jamais fait."
        )

    lance = corps(install, "pub fn lance_le_montage_differe(")
    if lance is None:
        fautes.append(
            "installation.rs : lance_le_montage_differe() a disparu ; le bureau "
            "n'a plus de point de declenchement."
        )
    else:
        if "spawn_noyau" not in lance:
            fautes.append(
                "installation.rs : le montage differe ne part plus dans son "
                "propre fil ; le compositeur reprend sur sa pile et dans son "
                "quantum tout ce que fait le disque."
            )
        # Le REPLI EN LIGNE a ete supprime, et il ne doit pas revenir : voir
        # `tools/verifie-montage-hors-compositeur.py`, qui porte cette regle et
        # explique pourquoi un systeme qui ne peut plus creer de tache est
        # precisement celui qu'il ne faut pas figer sur son fil graphique.
        if "MONTAGES_REFUSES" not in lance:
            fautes.append(
                "installation.rs : un refus de lancement n'est plus compte ; la "
                "persistance disparaitrait en silence sous pression memoire."
            )


def regle_verification_externe(fautes):
    """L'ESP telle qu'une VRAIE installation la produit, vue par fsck et mtools."""
    manquants = [o for o in ("fsck.fat", "mdir") if shutil.which(o) is None]
    if manquants or shutil.which("rustc") is None:
        print(
            "installation : %s absent(s), verification externe non executee"
            % ", ".join(manquants or ["rustc"])
        )
        return
    with tempfile.TemporaryDirectory() as brut:
        dossier = Path(brut)
        binaire = dossier / "test_installation"
        compilation = subprocess.run(
            ["rustc", "--edition", "2021", "--test", "-o", str(binaire), str(TEST)],
            capture_output=True,
            text=True,
        )
        if compilation.returncode != 0:
            fautes.append(
                "test_installation.rs ne compile pas :\n%s" % compilation.stderr[:800]
            )
            return
        image = dossier / "esp.img"
        execution = subprocess.run(
            [str(binaire), "--test-threads=1", "ecrit_l_image"],
            capture_output=True,
            text=True,
            env={**os.environ, "BO_INSTALL_IMAGE": str(image)},
        )
        if execution.returncode != 0 or not image.exists():
            fautes.append("l'ESP installee n'a pas ete produite.")
            return
        fsck = subprocess.run(
            ["fsck.fat", "-n", "-v", str(image)], capture_output=True, text=True
        )
        sortie = fsck.stdout + fsck.stderr
        if fsck.returncode != 0:
            fautes.append(
                "fsck.fat refuse l'ESP qu'une installation produit :\n%s"
                % sortie[:1200]
            )
        elif "32 bit entries" not in sortie:
            fautes.append(
                "l'ESP installee n'est pas du FAT32 :\n%s" % sortie[:800]
            )
        liste = subprocess.run(
            ["mdir", "-i", str(image), "::/"],
            capture_output=True,
            text=True,
            env={**os.environ, "MTOOLS_SKIP_CHECK": "1"},
        )
        for nom in ("kernel-x86_64", "boot.json", "ramdisk"):
            if nom not in liste.stdout:
                fautes.append(
                    "mtools ne retrouve pas « %s » dans l'ESP installee ; le "
                    "chargeur d'amorcage ne le trouvera pas non plus.\n%s"
                    % (nom, liste.stdout[:600])
                )
        boot = subprocess.run(
            ["mdir", "-i", str(image), "::/EFI/BOOT"],
            capture_output=True,
            text=True,
            env={**os.environ, "MTOOLS_SKIP_CHECK": "1"},
        )
        if "BOOTX64" not in boot.stdout.upper():
            fautes.append(
                "l'ESP installee ne porte pas EFI/BOOT/BOOTX64.EFI ; le "
                "micrologiciel l'ignorera.\n%s" % boot.stdout[:600]
            )


def main():
    fautes = []
    for chemin in (
        BUREAU,
        INSTALL, DISPOSITION, PERSISTANCE, COMMANDES, STAGE2, BUILDER,
        STAGE_PS, PREPARE_PS, TEST,
    ):
        if not chemin.exists():
            fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
    if fautes:
        for faute in fautes:
            print("  - %s" % faute)
        return 1

    install = sans_commentaires(INSTALL.read_text(encoding="utf-8"))
    disposition = sans_commentaires(DISPOSITION.read_text(encoding="utf-8"))
    persistance = PERSISTANCE.read_text(encoding="utf-8")
    commandes = sans_commentaires(COMMANDES.read_text(encoding="utf-8"))
    stage2 = sans_commentaires(STAGE2.read_text(encoding="utf-8"))
    builder = sans_commentaires(BUILDER.read_text(encoding="utf-8"))
    stage_ps = STAGE_PS.read_text(encoding="utf-8")
    prepare_ps = PREPARE_PS.read_text(encoding="utf-8")
    bureau = sans_commentaires(BUREAU.read_text(encoding="utf-8"))

    regle_zone_de_persistance(disposition, persistance, fautes)
    regle_esp_plafonnee(disposition, fautes)
    regle_rien_sans_demande(commandes, fautes)
    regle_disque_occupe(install, fautes)
    regle_liste_fermee(install, fautes)
    regle_charge_obligatoire(install, fautes)
    regle_fenetre_avant_enregistrement(install, fautes)
    regle_fenetre_bornee(install, fautes)
    regle_chaine_de_construction(builder, stage_ps, prepare_ps, fautes)
    regle_montage_differe(stage2, install, bureau, fautes)
    regle_verification_externe(fautes)

    if fautes:
        print("installation : %d probleme(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print(
        "installation : rien n'est ecrit sans demande, un disque occupe est "
        "refuse, la charge est obligatoire et l'archive la porte, la fenetre "
        "est bornee, et fsck.fat accepte l'ESP produite"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
