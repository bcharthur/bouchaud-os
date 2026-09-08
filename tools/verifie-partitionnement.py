#!/usr/bin/env python3
"""Ce que la table de partitions et l'ESP ne doivent jamais redevenir.

Une table de partitions et un systeme de fichiers d'amorcage sont du code
qu'on ne met pas au point sur la cible : ils s'ecrivent une fois, et s'ils
sont faux, la machine ne demarre plus et n'a plus rien a dire. Les regles
ci-dessous portent donc sur les proprietes dont la violation est SILENCIEUSE.

Chaque regle a ete mutee.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
GPT = RACINE / "src/fs/gpt.rs"
FAT = RACINE / "src/fs/fat32.rs"
TEST_GPT = RACINE / "tools/fs/test_gpt.rs"
TEST_FAT = RACINE / "tools/fs/test_fat32.rs"


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
        sortie.append(c)
        i += 1
    return "".join(sortie)


def corps(source, signature):
    """Le corps de la FONCTION dont la signature est donnee.

    Une signature peut apparaitre d'abord comme DECLARATION anticipee. On les
    distingue par le point-virgule qui la termine -- mais SEULEMENT celui qui
    est a profondeur nulle : `-> [u8; 16]` en contient un, et une regle qui le
    prenait pour une fin de declaration cherchait la fonction suivante sans le
    dire, puis rapportait « introuvable » sur une fonction bien presente.
    """
    debut = 0
    while True:
        trouve = source.find(signature, debut)
        if trouve < 0:
            return None
        ouvrante = -1
        declaration = -1
        profondeur = 0
        # On repart du DEBUT de la signature : certaines « signatures »
        # recherchees incluent deja leur accolade (`struct Champ {`), et
        # repartir apres elle ferait manquer la seule accolade qui compte.
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


def regle_somme_entete(gpt, fautes):
    """La somme de l'en-tete se calcule son propre champ a zero.

    Une somme calculee sur elle-meme est toujours fausse. Le micrologiciel
    bascule alors sur la copie de secours sans rien dire, et le disque demarre
    une fois sur deux selon l'ordre dans lequel on a ecrit.
    """
    bloc = corps(gpt, "pub fn encode_entete(")
    if bloc is None:
        fautes.append("gpt.rs : encode_entete introuvable.")
        return
    if "crc32(&bloc[0..TAILLE_ENTETE])" not in bloc:
        fautes.append(
            "gpt.rs : la somme de l'en-tete ne porte plus sur ses seuls "
            "octets utiles."
        )
    # La somme doit etre posee APRES son calcul, sur un champ encore nul.
    calcul = bloc.find("let somme")
    pose = bloc.find("bloc[16..20]")
    if calcul < 0 or pose < 0 or pose < calcul:
        fautes.append(
            "gpt.rs : le champ de somme est rempli AVANT le calcul ; la somme "
            "porterait alors sur elle-meme."
        )
    verif = corps(gpt, "pub fn decode_entete(")
    if verif is None or "copie[16..20].fill(0)" not in verif:
        fautes.append(
            "gpt.rs : decode_entete ne remet plus le champ de somme a zero "
            "avant de verifier ; aucun en-tete valide ne passerait."
        )


def regle_secours_echangee(gpt, fautes):
    """La copie de secours n'est pas une copie.

    `MyLBA` et `AlternateLBA` y sont echanges, et le tableau designe est
    l'autre. Une copie octet pour octet passe le controle de somme -- elle a
    ete calculee ailleurs -- et fait pointer la secours sur le tableau
    primaire, donc sur des blocs qu'on peut ecraser.
    """
    bloc = corps(gpt, "pub fn ecrit_table<S: Support>(")
    if bloc is None:
        fautes.append("gpt.rs : ecrit_table introuvable.")
        return
    secours = re.search(r"let secours = Entete \{(.*?)\};", bloc, re.S)
    if secours is None:
        fautes.append("gpt.rs : la copie de secours n'est plus construite a part.")
        return
    champs = secours.group(1)
    for champ in ("mon_lba", "autre_lba", "tableau_lba"):
        if champ not in champs:
            fautes.append(
                "gpt.rs : la secours ne redefinit plus %s ; elle designerait "
                "alors le tableau primaire, donc des blocs ecrasables." % champ
            )


def regle_mbr_en_dernier(gpt, fautes):
    """Le MBR de protection part apres les deux en-tetes.

    L'ecrire en premier annoncerait une table GPT pendant les millisecondes ou
    elle n'existe pas encore, et une coupure a cet instant laisserait un
    disque qui se declare partitionne sans l'etre.
    """
    bloc = corps(gpt, "pub fn ecrit_table<S: Support>(")
    if bloc is None:
        return
    mbr = bloc.find("mbr_protecteur(")
    primaire = bloc.find("encode_entete(&primaire")
    secours = bloc.find("encode_entete(&secours")
    if mbr < 0 or primaire < 0 or secours < 0:
        fautes.append("gpt.rs : une des trois ecritures de table a disparu.")
        return
    if mbr < primaire or mbr < secours:
        fautes.append(
            "gpt.rs : le MBR de protection est ecrit AVANT les en-tetes ; une "
            "coupure entre les deux laisse un disque qui se declare "
            "partitionne sans l'etre."
        )


def regle_zone_utilisable(gpt, fautes):
    """Une partition ne deborde jamais sur les tables.

    Le dernier bloc utilisable PRECEDE le tableau de secours. L'y inclure
    ferait ecraser la table par la derniere partition -- et le disque ne
    demarrerait plus apres le premier remplissage.
    """
    bloc = corps(gpt, "pub fn disposition(")
    if bloc is None:
        fautes.append("gpt.rs : disposition introuvable.")
        return
    if "tableau_secours - 1" not in bloc:
        fautes.append(
            "gpt.rs : le dernier bloc utilisable ne precede plus le tableau "
            "de secours."
        )
    controle = corps(gpt, "pub fn ecrit_table<S: Support>(")
    if controle is None:
        return
    for attendu in ("premier_utilisable", "dernier_utilisable", "PartitionHorsZone"):
        if attendu not in controle:
            fautes.append(
                "gpt.rs : ecrit_table ne verifie plus les bornes (%s absent)." % attendu
            )
    if "PartitionsQuiSeChevauchent" not in controle:
        fautes.append(
            "gpt.rs : deux partitions qui se recouvrent d'un seul bloc "
            "corrompent les deux systemes de fichiers ; le controle a disparu."
        )


def regle_guid_mixte(gpt, fautes):
    """Un GUID GPT n'est pas ecrit dans l'ordre du texte."""
    bloc = corps(gpt, "pub fn octets(&self)")
    if bloc is None:
        fautes.append("gpt.rs : l'encodage d'un GUID a disparu.")
        return
    for champ in ("d1.to_le_bytes", "d2.to_le_bytes", "d3.to_le_bytes"):
        if champ not in bloc:
            fautes.append(
                "gpt.rs : les trois premiers champs d'un GUID doivent etre "
                "petit-boutistes (%s absent) ; sinon le micrologiciel ne "
                "reconnait pas le type de partition." % champ
            )


def regle_fat32_est_bien_du_fat32(fat, fautes):
    """Un volume sous le seuil d'amas n'est PAS du FAT32.

    Ce n'est pas une convention : c'est la regle qui permet a un lecteur de
    determiner le type. Un volume de soixante mille amas formate « en FAT32 »
    sera lu comme du FAT16 par un micrologiciel conforme.
    """
    if "AMAS_MINIMUM_FAT32" not in fat:
        fautes.append("fat32.rs : le seuil d'amas de FAT32 a disparu.")
        return
    seuil = re.search(r"AMAS_MINIMUM_FAT32:\s*u32\s*=\s*([0-9_]+)", fat)
    if seuil is None or int(seuil.group(1).replace("_", "")) != 65525:
        fautes.append(
            "fat32.rs : le seuil d'amas n'est plus 65 525, la valeur qui "
            "distingue FAT32 de FAT16."
        )
    bloc = corps(fat, "pub fn choisit_amas(")
    if bloc is None or "AMAS_MINIMUM_FAT32" not in bloc:
        fautes.append(
            "fat32.rs : le choix de la taille d'amas ne verifie plus le seuil."
        )
    amorce = corps(fat, "pub fn secteur_amorce(")
    if amorce is None:
        fautes.append("fat32.rs : secteur_amorce introuvable.")
        return
    for decalage in ("s[17..19]", "s[19..21]", "s[22..24]"):
        motif = re.search(re.escape(decalage) + r"\.copy_from_slice\(&([^)]+)\)", amorce)
        if motif is None:
            fautes.append("fat32.rs : le champ %s a disparu du secteur d'amorcage." % decalage)
        elif not motif.group(1).startswith("0u16"):
            fautes.append(
                "fat32.rs : %s doit etre NUL en FAT32 ; non nul, le volume est "
                "lu comme du FAT16 et le micrologiciel n'y trouve rien."
                % decalage
            )


def regle_noms_longs(fat, fautes):
    """Le chargeur cherche des noms qui ne tiennent pas en 8.3.

    `kernel-x86_64` et `boot.json` deviennent `KERNEL-X` et `BOOT.JSO` sans
    entrees de nom long -- et la machine ne demarre pas, sans un mot.
    """
    bloc = corps(fat, "pub fn entrees_nom_long(")
    if bloc is None:
        fautes.append("fat32.rs : les entrees de nom long ont disparu.")
        return
    if ".rev()" not in bloc:
        fautes.append(
            "fat32.rs : les entrees de nom long ne sont plus ecrites a "
            "l'envers ; l'ordre naturel donne un nom lu a l'envers, donc un "
            "fichier introuvable."
        )
    if "somme_nom_court" not in bloc:
        fautes.append(
            "fat32.rs : le nom long ne porte plus la somme de son nom court ; "
            "une somme absente ou fausse fait IGNORER tout le nom long."
        )
    if "0x40" not in bloc:
        fautes.append(
            "fat32.rs : la marque de derniere entree de nom long a disparu."
        )
    # Le meme critere des deux cotes : deux criteres qui divergent donnent un
    # fichier ecrit sous un nom que personne ne cherche.
    court = corps(fat, "pub fn nom_court(")
    if court is None or "tient_en_8_3(nom)" not in court:
        fautes.append(
            "fat32.rs : nom_court n'utilise plus le MEME critere que "
            "tient_en_8_3 ; un nom juge court d'un cote et long de l'autre "
            "donne un fichier ecrit sous un nom que personne ne cherche."
        )
    for nom in ("ecrit_fichier", "cree_repertoire"):
        bloc = corps(fat, "pub fn %s(" % nom)
        if bloc is None or "entrees_nom_long" not in bloc:
            fautes.append("fat32.rs : %s n'ecrit plus de nom long." % nom)


def regle_toutes_les_fats(fat, fautes):
    """Une entree de FAT est ecrite dans TOUTES les copies.

    N'en ecrire qu'une donne un volume que le systeme vivant lit sans probleme
    et qu'un outil de reparation « repare » en recopiant la copie perimee.
    """
    bloc = corps(fat, "fn pose_fat(&mut self,")
    if bloc is None:
        fautes.append("fat32.rs : pose_fat introuvable.")
        return
    if "self.params.fats" not in bloc:
        fautes.append(
            "fat32.rs : pose_fat n'ecrit plus dans toutes les copies de la FAT."
        )


def regle_espace_libre_honnete(fat, fautes):
    """Le compte d'amas libres est juste, ou declare inconnu.

    Un nombre PERIME est une incoherence que `fsck.fat` signale et propose de
    « corriger » -- exactement le genre de reparation qu'on ne veut pas qu'un
    outil tiers entreprenne sur une ESP.
    """
    if "pub const INCONNU" not in fat:
        fautes.append("fat32.rs : la valeur « inconnu » du secteur d'information a disparu.")
    formate = corps(fat, "pub fn formate<'a, S: Support>(")
    if formate is None:
        fautes.append("fat32.rs : formate introuvable.")
    elif "secteur_info(&params, INCONNU" not in formate:
        fautes.append(
            "fat32.rs : le formatage annonce un nombre d'amas libres que les "
            "ecritures suivantes rendront faux ; il doit annoncer INCONNU."
        )
    termine = corps(fat, "pub fn termine(&mut self)")
    if termine is None or "secteur_info" not in termine:
        fautes.append(
            "fat32.rs : rien n'ecrit plus le compte reel d'amas libres."
        )


def regle_decalage_de_partition(fat, fautes):
    """Tout est ecrit relativement au premier bloc de la partition.

    Melanger l'origine du volume et celle du disque est la faute qui ecrit un
    systeme de fichiers parfaitement forme par-dessus la table de partitions.
    """
    for nom in ("fn lit_secteur(&mut self,", "fn ecrit_secteur(&mut self,"):
        bloc = corps(fat, nom)
        if bloc is None:
            fautes.append("fat32.rs : %s introuvable." % nom)
        elif "self.decalage +" not in bloc:
            fautes.append(
                "fat32.rs : %s n'ajoute plus le decalage de la partition ; le "
                "systeme de fichiers s'ecrirait par-dessus la table de "
                "partitions." % nom
            )


def regle_preuves(test_gpt, test_fat, fautes):
    for attendu in (
        "la_secours_n_est_pas_une_copie_de_la_primaire",
        "le_disque_bascule_sur_la_secours_quand_la_primaire_est_detruite",
        "deux_partitions_qui_se_chevauchent_sont_refusees",
        "le_mbr_est_ecrit_en_dernier",
    ):
        if attendu not in test_gpt:
            fautes.append("test_gpt.rs : la preuve « %s » a disparu." % attendu)
    for attendu in (
        "les_deux_noms_du_chargeur_ne_tiennent_pas_en_8_3",
        "une_esp_relue_par_un_lecteur_independant_contient_ses_fichiers",
        "les_entrees_de_nom_long_sont_ecrites_a_l_envers",
        "un_volume_trop_petit_pour_du_vrai_fat32_est_refuse",
        "le_volume_est_ecrit_a_son_decalage_et_pas_ailleurs",
    ):
        if attendu not in test_fat:
            fautes.append("test_fat32.rs : la preuve « %s » a disparu." % attendu)


def main():
    fautes = []
    for chemin in (GPT, FAT, TEST_GPT, TEST_FAT):
        if not chemin.exists():
            fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
    if fautes:
        for faute in fautes:
            print("  - %s" % faute)
        return 1

    gpt = sans_commentaires(GPT.read_text(encoding="utf-8"))
    fat = sans_commentaires(FAT.read_text(encoding="utf-8"))
    test_gpt = TEST_GPT.read_text(encoding="utf-8")
    test_fat = TEST_FAT.read_text(encoding="utf-8")

    regle_somme_entete(gpt, fautes)
    regle_secours_echangee(gpt, fautes)
    regle_mbr_en_dernier(gpt, fautes)
    regle_zone_utilisable(gpt, fautes)
    regle_guid_mixte(gpt, fautes)
    regle_fat32_est_bien_du_fat32(fat, fautes)
    regle_noms_longs(fat, fautes)
    regle_toutes_les_fats(fat, fautes)
    regle_espace_libre_honnete(fat, fautes)
    regle_decalage_de_partition(fat, fautes)
    regle_preuves(test_gpt, test_fat, fautes)

    if fautes:
        print("partitionnement : %d regle(s) violee(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print(
        "partitionnement : somme d'en-tete a champ nul, secours echangee, MBR "
        "en dernier, zone utilisable bornee, noms longs a l'envers, deux FAT "
        "ecrites, espace libre juste ou inconnu"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
