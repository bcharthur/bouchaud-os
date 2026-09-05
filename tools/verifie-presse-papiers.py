#!/usr/bin/env python3
"""Verifie les regles du presse-papiers du bureau.

CE QUI EST EN JEU
-----------------
Un presse-papiers est l'endroit ou passent les secrets d'un utilisateur : un
mot de passe sorti d'un gestionnaire, une phrase de recuperation, un jeton
d'authentification, l'adresse d'un virement. Les deux fautes classiques sont
connues depuis X11 et n'ont jamais rien casse de visible :

  * un programme en arriere-plan LIT le presse-papiers en boucle et recolte
    tout ce que l'utilisateur copie ailleurs ;
  * un programme en arriere-plan l'ECRIT et remplace ce que l'utilisateur
    vient de copier par autre chose -- l'adresse d'un virement par une autre.

La conception ferme les deux par la meme regle : seul le client qui a le FOYER
participe. Elle ferme la lecture d'une facon particuliere, et c'est ce que ce
verificateur garde : il n'existe aucun message de lecture dans le protocole.
Le contenu est POUSSE au client focalise, jamais demande. On ne peut pas
oublier de garder un chemin qui n'existe pas -- encore faut-il qu'il continue
de ne pas exister.

LES REGLES
----------
1. L'ecriture est refusee a un client sans foyer.

2. Le contenu n'est POUSSE qu'au client qui a le foyer.

3. Le seul envoi de `PressePapiers` part de la synchronisation, jamais du
   traitement d'un message client. Un envoi declenche par un message serait
   un chemin de lecture, quel que soit le nom du message qui le declenche.

4. Le contenu est BORNE, et la borne est celle du protocole. Le decodeur du
   noyau rejette le flux entier au-dela de `CHARGE_MAX` : une seconde borne
   ecrite a la main finirait par diverger, et c'est alors le canal GUI de la
   fenetre qui tombe.

5. Le chrome tronque AVANT d'envoyer. Meme raison, de l'autre cote du fil :
   une selection de six kibioctets ne doit pas couper la fenetre.

6. Un texte colle dans un champ du chrome passe par `Champ::colle`, qui filtre
   les octets non imprimables. Le contenu vient possiblement d'une autre
   application : une chaine portant un saut de ligne puis une seconde adresse,
   collee dans une barre qui les accepterait toutes deux, montre une adresse
   et en visite une autre.

7. Un texte colle dans le DOCUMENT est converti avec le caractere de
   remplacement. `Utf16String::from_utf8` AFFIRME la validite de son entree ;
   affirmer sur un contenu venu d'un autre processus est une panne qui attend.

8. Les quatre operations d'edition et les deux entrees de l'API Clipboard sont
   branchees. Un raccourci qui ne fait rien est pire que pas de raccourci.

9. Le banc d'essai hote existe. Il exerce la borne et la generation, que rien
   d'autre n'exerce.

Code de retour : 0 si les neuf regles sont respectees.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent

MODULE = RACINE / "src" / "gui" / "presse_papiers.rs"
CLIENT = RACINE / "src" / "gui" / "client.rs"
PROTOCOLE = RACINE / "src" / "gui" / "protocole.rs"
CHROME = RACINE / "tools" / "ladybird" / "chrome" / "BouchaudChrome.h"
V19 = RACINE / "tools" / "ladybird" / "prepare-v19-navigateur.py"
BANC = RACINE / "tools" / "gui" / "test_presse_papiers.rs"


def texte(chemin, fautes):
    if not chemin.exists():
        fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
        return None
    return chemin.read_text(encoding="utf-8")


def sans_commentaires(source):
    """Le meme texte, commentaires de ligne retires, chaines preservees.

    Les regles ci-dessous cherchent des APPELS, pas des mentions. Or ce depot
    ecrit beaucoup de commentaires, et ils citent le code qu'ils expliquent :
    sans ce depouillement, une regle se laisse satisfaire par la phrase qui
    decrit ce que le code faisait avant. C'est arrive a la premiere version --
    l'ecretage du chrome pouvait disparaitre sans que rien ne bouge, parce que
    le commentaire au-dessus nommait encore `CHARGE_MAX`.

    Seules les chaines a guillemets doubles sont suivies : elles suffisent a
    proteger `"https://"` et ses semblables, et les litteraux de caractere ne
    sont pas suivis parce qu'en Rust une apostrophe ouvre aussi une duree de
    vie -- `<'a>` -- qu'un analyseur naif prendrait pour une chaine.
    """
    sortie = []
    index = 0
    taille = len(source)
    while index < taille:
        caractere = source[index]
        if caractere == '"':
            sortie.append(caractere)
            index += 1
            while index < taille:
                sortie.append(source[index])
                if source[index] == "\\" and index + 1 < taille:
                    sortie.append(source[index + 1])
                    index += 2
                    continue
                if source[index] == '"':
                    index += 1
                    break
                index += 1
            continue
        if source.startswith("//", index):
            fin = source.find("\n", index)
            index = taille if fin < 0 else fin
            continue
        sortie.append(caractere)
        index += 1
    return "".join(sortie)


def corps(source, signature, ouvrant="{", fermant="}"):
    """Le corps qui suit `signature`, delimiteurs equilibres."""
    debut = source.find(signature)
    if debut < 0:
        return None
    ouvre = source.find(ouvrant, debut)
    if ouvre < 0:
        return None
    profondeur = 0
    for index in range(ouvre, len(source)):
        if source[index] == ouvrant:
            profondeur += 1
        elif source[index] == fermant:
            profondeur -= 1
            if profondeur == 0:
                return source[ouvre : index + 1]
    return None


def regle_ecriture_focalisee(client, fautes):
    """1. L'ecriture est refusee a un client sans foyer."""
    bloc = corps(client, "Genre::PressePapiersEcrit =>")
    if bloc is None:
        fautes.append(
            "client.rs : le traitement de `PressePapiersEcrit` a disparu ; un "
            "client ne pourrait plus rien copier vers le bureau."
        )
        return
    if "a_le_focus" not in bloc:
        fautes.append(
            "client.rs : `PressePapiersEcrit` est accepte sans verifier le "
            "foyer. Un programme d'arriere-plan pourrait alors remplacer ce "
            "que l'utilisateur vient de copier, sans que rien ne le montre."
        )


def regle_poussee_focalisee(client, fautes):
    """2 et 3. Le contenu ne part qu'a un client focalise, et depuis un seul point."""
    bloc = corps(client, "pub fn synchronise_presse_papiers(")
    if bloc is None:
        fautes.append(
            "client.rs : `synchronise_presse_papiers` a disparu ; plus rien "
            "ne remettrait le presse-papiers a un client."
        )
        return
    if "if !focus" not in bloc:
        fautes.append(
            "client.rs : `synchronise_presse_papiers` ne s'arrete plus quand "
            "le client n'a pas le foyer. Un programme d'arriere-plan verrait "
            "alors passer tout ce que l'utilisateur copie ailleurs."
        )
    if "generation" not in bloc:
        fautes.append(
            "client.rs : la poussee ne consulte plus la generation ; elle "
            "recopierait quatre kibioctets par client et par trame."
        )

    envois = [
        ligne.strip()
        for ligne in client.splitlines()
        if "Genre::PressePapiers," in ligne.split("//", 1)[0]
    ]
    if len(envois) != 1:
        fautes.append(
            "client.rs : %d envois de `Genre::PressePapiers` au lieu d'un "
            "seul. Le contenu ne doit partir que de la synchronisation : un "
            "envoi declenche par un message recu serait un chemin de LECTURE, "
            "quel que soit le nom du message qui le declenche." % len(envois)
        )


def regle_pas_de_lecture(protocole, fautes):
    """3 (suite). Aucun message de lecture n'est declare."""
    bloc = re.search(r"pub enum Genre \{(.*?)\n\}", protocole, re.S)
    if not bloc:
        fautes.append("protocole.rs : enum Genre introuvable.")
        return
    noms = re.findall(r"^\s*(\w+) = ", bloc.group(1), re.M)
    suspects = [
        nom
        for nom in noms
        if "PressePapiers" in nom and nom not in ("PressePapiersEcrit", "PressePapiers")
    ]
    if suspects:
        fautes.append(
            "protocole.rs : le protocole declare %s. Le presse-papiers est "
            "POUSSE au client focalise, jamais demande : un message de demande "
            "rouvre exactement le chemin que cette conception ferme."
            % ", ".join(suspects)
        )


def regle_borne(module, fautes):
    """4. Le contenu est borne, et la borne est celle du protocole."""
    if "CHARGE_MAX" not in module:
        fautes.append(
            "presse_papiers.rs : la capacite ne derive plus de "
            "`protocole::CHARGE_MAX`. Deux bornes independantes pour une seule "
            "contrainte finissent par diverger, et c'est alors le canal GUI de "
            "la fenetre qui tombe."
        )
    bloc = corps(module, "pub fn ecrit(")
    if bloc is None:
        fautes.append("presse_papiers.rs : `ecrit` introuvable.")
        return
    if "min(" not in bloc or "CAPACITE" not in bloc:
        fautes.append(
            "presse_papiers.rs : `ecrit` ne borne plus le contenu. Un client "
            "pourrait faire grossir la memoire du noyau a volonte."
        )
    if "generation" not in bloc:
        fautes.append(
            "presse_papiers.rs : `ecrit` ne fait plus avancer la generation ; "
            "aucun client ne se verrait pousser le nouveau contenu."
        )


def regle_chrome(chrome, fautes):
    """5, 6 et 8 cote chrome."""
    envoi = corps(chrome, "inline void copie_vers_le_presse_papiers(")
    if envoi is None:
        fautes.append("BouchaudChrome.h : `copie_vers_le_presse_papiers` introuvable.")
    else:
        if "CHARGE_MAX" not in envoi:
            fautes.append(
                "BouchaudChrome.h : la copie n'est plus bornee a `CHARGE_MAX`. "
                "Le decodeur du noyau REJETTE le flux entier au-dela -- il ne "
                "tronque pas -- donc une grande selection couperait le canal "
                "GUI de la fenetre."
            )
        if "Genre::PressePapiersEcrit" not in envoi:
            fautes.append(
                "BouchaudChrome.h : la copie ne sort plus de la fenetre ; le "
                "presse-papiers redeviendrait un tampon interne."
            )

    colle = corps(chrome, "inline void colle_le_presse_papiers(")
    if colle is None:
        fautes.append("BouchaudChrome.h : `colle_le_presse_papiers` introuvable.")
    elif ".colle(" not in colle:
        fautes.append(
            "BouchaudChrome.h : le collage dans un champ ne passe plus par "
            "`Champ::colle`, qui filtre les octets non imprimables. Le contenu "
            "vient possiblement d'une autre application."
        )
    if colle is not None:
        # Il y a plusieurs champs, et la regle porte sur TOUS. Se contenter de
        # « `.colle(` apparait quelque part » laisse un champ passer au travers,
        # et ce sera celui qu'on a le moins regarde -- la barre d'adresse, ou
        # l'octet filtre compte le plus.
        for interdit in (".pose(", ".texte.insert(", "texte.append("):
            if interdit in colle:
                fautes.append(
                    "BouchaudChrome.h : `colle_le_presse_papiers` ecrit dans un "
                    "champ par `%s`, ce qui contourne le filtre de "
                    "`Champ::colle`." % interdit
                )

    filtre = corps(chrome, "void colle(StringView valeur)")
    if filtre is None:
        fautes.append("BouchaudChrome.h : `Champ::colle` introuvable.")
    elif "0x20" not in filtre or "0x7f" not in filtre:
        fautes.append(
            "BouchaudChrome.h : `Champ::colle` ne filtre plus les octets non "
            "imprimables. Une chaine portant un saut de ligne puis une seconde "
            "adresse montrerait une adresse et en visiterait une autre."
        )

    raccourcis = corps(chrome, "inline bool raccourci_navigateur(")
    if raccourcis is not None:
        for appel, quoi in (
            ("selectionne_tout_le_foyer()", "Ctrl+A"),
            ("copie_la_selection(", "Ctrl+C / Ctrl+X"),
            ("colle_le_presse_papiers()", "Ctrl+V"),
        ):
            if appel not in raccourcis:
                fautes.append(
                    "BouchaudChrome.h : %s ne fait plus rien." % quoi
                )


def regle_v19(v19, fautes):
    """7 et 8 cote portage."""
    for symbole, quoi in (
        ("chrome.on_select_all = [", "la selection totale du document"),
        ("chrome.on_copy = [", "la copie depuis le document"),
        ("chrome.on_cut = [", "le couper depuis le document"),
        ("chrome.on_paste = [", "le collage dans le document"),
        ("set_presse_papiers_du_document(", "l'ecriture par l'API Clipboard"),
        ("retrieved_clipboard_entries(", "la lecture par l'API Clipboard"),
    ):
        if symbole not in v19:
            fautes.append(
                "prepare-v19-navigateur.py : %s n'est plus branchee." % quoi
            )
    if "from_utf8_with_replacement_character" not in v19:
        fautes.append(
            "prepare-v19-navigateur.py : le collage dans le document ne "
            "convertit plus avec le caractere de remplacement. "
            "`Utf16String::from_utf8` AFFIRME la validite de son entree, et "
            "ce texte vient d'un autre processus : l'affirmation est une "
            "panne qui attend."
        )


def main():
    fautes = []
    module = texte(MODULE, fautes)
    client = texte(CLIENT, fautes)
    protocole = texte(PROTOCOLE, fautes)
    chrome = texte(CHROME, fautes)
    v19 = texte(V19, fautes)
    texte(BANC, fautes)

    if client is not None:
        code = sans_commentaires(client)
        regle_ecriture_focalisee(code, fautes)
        regle_poussee_focalisee(code, fautes)
    if protocole is not None:
        regle_pas_de_lecture(sans_commentaires(protocole), fautes)
    if module is not None:
        regle_borne(sans_commentaires(module), fautes)
    if chrome is not None:
        regle_chrome(sans_commentaires(chrome), fautes)
    if v19 is not None:
        regle_v19(sans_commentaires(v19), fautes)

    if fautes:
        print("presse-papiers : %d regle(s) violee(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print("presse-papiers : pousse au client focalise, borne au message, "
          "colle filtre, aucun chemin de lecture")
    return 0


if __name__ == "__main__":
    sys.exit(main())
