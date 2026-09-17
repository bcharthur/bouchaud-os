#!/usr/bin/env python3
"""Garde-fou : produire une trace ne doit plus dependre du support USB.

# Le defaut, mesure sur trois sessions physiques

`kernel::blackbox::append()` ecrivait PHYSIQUEMENT un enregistrement de quatre
kibioctets sur la cle USB a chaque appel : prise du verrou du pilote xHCI,
trois transferts Bulk synchrones, deux attentes d'evenement bornees a une
demi-seconde chacune.

Les trois archives s'arretent entre 7,3 s et 8,2 s, a l'instant ou les
services du navigateur commencent a lire la cle d'amorcage. Toutes les causes
candidates ont ete mesurees et eliminees : tambour plein (73 enregistrements
sur 8192), echec d'ecriture (`bb_failures=0`), famine du verrou
(`equite_enregistreur_sauts=0`), support retire (jamais), ecrasement
(`first_seq=1`), extracteur, architecture, nombre de coeurs.

Ce qui restait etait la FORME du chemin : un diagnostic qui depend du
peripherique qu'il observe ne peut pas rapporter la panne de ce peripherique.

# Ce qui est verifie ici

1. `kernel/debug/bobine.rs` reste PUR -- aucun `use crate::`, aucun `unsafe`,
   aucune horloge. C'est ce qui permet a une machine hote de mettre a
   l'epreuve la discipline d'anneau, que la machine cible ne montre pas.
2. L'invariant `OCTETS = DESCRIPTEURS * PAYLOAD_MAX` tient. Il supprime la
   SECONDE cause de perte -- le lapage de l'anneau d'octets -- et rend le
   compte d'ecrasements exact. Le relacher ramenerait une perte qui ne se
   calcule pas sans parcourir l'anneau.
3. `append` ne touche plus le pilote USB. Ni `xhci_active`, ni un verrou, ni
   une attente : c'est la regle entiere.
4. La publication a lieu APRES la copie. Publier avant exposerait au lecteur
   les octets de l'enregistrement precedent, sans que rien ne le lui dise.
5. Aucune ecriture USB PERIODIQUE ne subsiste : `poll()` ne vide plus rien
   vers le support. C'est une experience A/B explicite -- tant qu'une
   ecriture periodique subsiste, un enregistrement qui s'arrete reste
   ambigu.
6. Le vidage ATTEND le verrou au lieu de le tenter. Depuis que le vidage n'a
   lieu qu'a l'extinction, renoncer ne coute plus un quart de seconde : il
   coute toute la trace. Mesure au banc : `sautes_occupe=21`,
   `ecritures=0`, mille sept cent vingt-cinq enregistrements en RAM et pas
   un seul sur la cle.
7. Le vidage ecrit par LOTS contigus, et coupe au tour de l'anneau du
   support. Ecrire le lot entier au premier LBA apres un tour ecraserait des
   enregistrements etrangers avec des sommes de controle valides -- pire
   qu'un trou.
8. La suite hote existe et confronte le compteur de perte a la formule
   fermee.
9. CHAQUE motif d'echec a un code. L'ecran d'extinction de la session
   physique du 17 septembre affichait `err=0xff` : le motif etait tombe dans
   le `_` du filtre, parce que les motifs introduits avec la reprise BOT
   n'avaient pas ete ajoutes a la table. Le chiffre le plus important de
   l'ecran ne designait rien, et il a fallu retrouver la reponse dans le
   journal serie -- qui, cette fois, existait.
10. Un STALL en phase de donnees n'est PAS traite comme une panne de
   transport. La specification Bulk-Only est explicite : le peripherique qui
   refuse la phase de donnees arrete son point et attend qu'on vienne lire
   son CSW. Le traiter comme un transport casse a coute cinquante
   reinitialisations de classe completes sur la session physique, et c'est ce
   cout qui a empeche le vidage final d'aller au bout.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]

BOBINE = RACINE / "src/kernel/debug/bobine.rs"
BLACKBOX = RACINE / "src/kernel/debug/blackbox.rs"
STOCKAGE = RACINE / "src/drivers/usb/blackbox_storage.rs"
TEST = RACINE / "tools/platform/test_bobine.rs"


def code_seul(source):
    """La source privee de ses commentaires.

    Une garde qui lit la prose verifie ce que le code PRETEND, pas ce qu'il
    fait -- et la documentation de `bobine.rs` cite justement les jetons
    qu'elle interdit.
    """
    sans_blocs = re.sub(r"/\*.*?\*/", "", source, flags=re.S)
    return "\n".join(re.sub(r"//.*$", "", ligne) for ligne in sans_blocs.splitlines())


def corps(source, signature):
    """Le corps d'une fonction, du `fn` a la prochaine declaration de meme rang.

    Borner la fenetre est ce qui evite qu'une regle sur `append` se satisfasse
    d'une ligne trouvee cent lignes plus bas, dans une autre fonction.
    """
    debut = source.find(signature)
    if debut < 0:
        return None
    reste = source[debut + len(signature):]
    fin = re.search(r"\n(?:pub )?(?:const |static |fn |struct |impl |enum )", reste)
    return reste[: fin.start()] if fin else reste


def lit(chemin, fautes):
    if not chemin.exists():
        fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
        return None
    return chemin.read_text(encoding="utf-8")


def main():
    fautes = []
    bobine = lit(BOBINE, fautes)
    blackbox = lit(BLACKBOX, fautes)
    stockage = lit(STOCKAGE, fautes)
    test = lit(TEST, fautes)

    # 1. La purete, qui est ce qui rend la discipline verifiable.
    if bobine is not None:
        pur = code_seul(bobine)
        if "use crate::" in pur:
            fautes.append(
                "bobine.rs depend du reste du noyau. Il ne se compile plus "
                "seul avec `rustc --test`, et la discipline d'anneau "
                "redevient invrifiable -- or c'est la partie dont une faute "
                "ne produit aucun message sur la machine, seulement une "
                "charge utile melangee."
            )
        if "unsafe" in pur:
            fautes.append(
                "bobine.rs contient du `unsafe`. La discipline d'index doit "
                "se raisonner sans pointeur ; la copie, elle, vit dans "
                "blackbox.rs, ou elle est triviale."
            )
        if "monotonic_ns" in pur:
            fautes.append(
                "bobine.rs lit l'horloge lui-meme au lieu de la recevoir en "
                "parametre."
            )

        # 2. L'invariant qui rend la perte exacte.
        if not re.search(
            r"pub const OCTETS:\s*usize\s*=\s*DESCRIPTEURS\s*\*\s*PAYLOAD_MAX\s*;", pur
        ):
            fautes.append(
                "bobine.rs : `OCTETS` n'est plus derive de "
                "`DESCRIPTEURS * PAYLOAD_MAX`. Cet invariant est ce qui "
                "empeche l'anneau d'octets de laper avant l'anneau de "
                "descripteurs -- donc ce qui reduit la perte a UNE cause, "
                "celle qui se compte exactement. Le relacher ramene une "
                "perte qu'on ne peut plus annoncer."
            )
        reserve = corps(pur, "pub fn reserve(")
        if reserve is not None and "ecrases.fetch_add" not in reserve:
            fautes.append(
                "bobine.rs : `reserve` ne compte plus les ecrasements. Zero "
                "est la seule valeur qui autorise a lire la trace comme un "
                "recit complet ; sans ce compteur, on ne sait plus si elle "
                "en est un."
            )
        lis = corps(pur, "pub fn lis(")
        if lis is not None and lis.count("publie.load") < 2:
            fautes.append(
                "bobine.rs : `lis` ne verifie plus le numero DEUX fois. Entre "
                "les deux lectures, un producteur peut recycler le "
                "descripteur ; sans la seconde, on rendrait un enregistrement "
                "dont les champs viennent de deux enregistrements differents."
            )

    # 3 et 4. Le chemin chaud ne touche plus le peripherique.
    if blackbox is not None:
        pur = code_seul(blackbox)
        append = corps(pur, "fn append(kind: u16")
        if append is None:
            fautes.append("blackbox.rs : `append` est introuvable.")
        else:
            for jeton, pourquoi in (
                ("xhci", "le chemin du diagnostic repasserait par le "
                         "peripherique qu'il observe"),
                ("RUNTIME_BUSY", "poser un enregistrement redeviendrait "
                                 "dependant du verrou du pilote"),
                ("blackbox_storage_ready", "une machine sans cle ne "
                                           "produirait plus de trace du tout"),
                ("wait_", "une attente sur le chemin chaud est exactement ce "
                          "qui figeait l'entree"),
            ):
                if jeton in append:
                    fautes.append(
                        "blackbox.rs : `append` mentionne `%s` -- %s." % (jeton, pourquoi)
                    )
            copie = append.find("copie_dans_le_tambour")
            publie = append.find("BOBINE.publie")
            if copie < 0 or publie < 0:
                fautes.append(
                    "blackbox.rs : `append` ne copie plus puis ne publie plus. "
                    "Ce sont les deux moities de la publication atomique."
                )
            elif publie < copie:
                fautes.append(
                    "blackbox.rs : `append` publie AVANT de copier. Le lecteur "
                    "verrait alors les octets de l'enregistrement precedent, "
                    "et rien ne le lui dirait."
                )

        # 5. Aucune ecriture periodique ne subsiste.
        poll = corps(pur, "pub fn poll()")
        if poll is None:
            fautes.append("blackbox.rs : `poll` est introuvable.")
        else:
            for jeton, pourquoi in (
                ("vidange(", "une ecriture USB periodique reintroduirait "
                             "l'ambiguite qu'on cherche justement a lever"),
                ("flush_serial", "le journal serie vit deja dans son propre "
                                 "anneau RAM ; le recopier chasserait les "
                                 "echantillons, dont personne ne garde de copie"),
                ("flush_flight", "les evenements de vol vivent deja dans leur "
                                 "propre anneau RAM, meme raison"),
                ("blackbox_storage_ready", "une machine sans cle doit produire "
                                           "une trace RAM parfaitement valide"),
            ):
                if jeton in poll:
                    fautes.append(
                        "blackbox.rs : `poll` mentionne `%s` -- %s." % (jeton, pourquoi)
                    )

    # 6 et 7. Le vidage attend, et coupe au bon endroit.
    if stockage is not None:
        pur = code_seul(stockage)
        vidange = corps(pur, "pub fn blackbox_vidange_lot(")
        if vidange is None:
            fautes.append("blackbox_storage.rs : `blackbox_vidange_lot` est introuvable.")
        else:
            if "attends_le_pilote" not in vidange:
                fautes.append(
                    "blackbox_storage.rs : le vidage tente le verrou au lieu "
                    "de l'attendre. Mesure au banc : le fil de charge lisait "
                    "encore pendant l'extinction, `sautes_occupe=21`, "
                    "`ecritures=0` -- mille sept cent vingt-cinq "
                    "enregistrements en RAM et pas un seul sur la cle."
                )
            if "enregistreur_a_saute" not in vidange:
                fautes.append(
                    "blackbox_storage.rs : le vidage ne reclame plus le "
                    "passage. Sans cette reclamation, `avec_le_pilote_usb` ne "
                    "cede pas, et une lecture soutenue garde le verrou presque "
                    "en continu."
                )
        lot = corps(pur, "fn blackbox_vidange_un_support(")
        if lot is not None and "place != place_suivante" not in lot:
            fautes.append(
                "blackbox_storage.rs : le lot ne coupe plus au tour de "
                "l'anneau du support. Ecrire le lot entier au premier LBA "
                "ecraserait des enregistrements etrangers avec des sommes de "
                "controle VALIDES -- pire qu'un trou, parce qu'indetectable."
            )
        if "fn blackbox_append_record" in pur:
            fautes.append(
                "blackbox_storage.rs : `blackbox_append_record` est revenu. "
                "C'est le chemin par enregistrement, verrou pris et rendu a "
                "chaque fois, depuis le chemin chaud -- exactement ce que ce "
                "lot supprime."
            )

    # 9. Aucun motif d'echec ne tombe dans le `_`.
    if stockage is not None:
        pur = code_seul(stockage)
        table = corps(pur, "fn blackbox_error_code(error: &'static str) -> u64 {")
        if table is None:
            fautes.append("blackbox_storage.rs : la table des codes d'echec a disparu.")
        else:
            # Les motifs sont cherches dans le FICHIER, la table exclue : un
            # motif qui ne figure que dans la table n'est plus emis, et c'est
            # sans consequence.
            hors_table = pur.replace(table, "")
            motifs = set(re.findall(r'"(blackbox-[a-z-]+)"', hors_table))
            codes = set(re.findall(r'"(blackbox-[a-z-]+)" =>', table))
            manquants = sorted(motifs - codes)
            if manquants:
                fautes.append(
                    "blackbox_storage.rs : %d motif(s) d'echec sans code -- ils "
                    "tomberaient dans le `_` et s'afficheraient `err=0xff` a "
                    "l'extinction, exactement comme le 17 septembre : %s"
                    % (len(manquants), ", ".join(manquants))
                )

        # 10. Le STALL de la phase de donnees suit le protocole.
        donnees = corps(pur, "fn blackbox_bot_en_place(")
        # LE MOTIF EXACT, ARME COMPRIS.
        #
        # Chercher `"blackbox-bulk-stall" =>` ne trouvait rien : dans le code
        # l'arme s'ecrit `Err("blackbox-bulk-stall") =>`, avec la parenthese
        # entre les deux. La garde criait donc sur le code qu'elle defend.
        if donnees is not None and 'Err("blackbox-bulk-stall")' not in donnees:
            fautes.append(
                "blackbox_storage.rs : un STALL en phase de donnees redevient "
                "une panne de transport. La specification Bulk-Only dit que le "
                "peripherique qui refuse les donnees ARRETE son point et attend "
                "qu'on lise son CSW : debloquer suffit. Le confondre avec un "
                "transport casse a coute cinquante reinitialisations de classe "
                "sur la session physique, et le vidage final n'est pas alle au "
                "bout."
            )

    # 11. Le recul du lot ne repond qu'a un refus de TAILLE.
    if stockage is not None:
        pur = code_seul(stockage)
        ecriture = corps(pur, "fn blackbox_ecris_lot_avec_reprise(")
        # CHAQUE recul doit etre garde, pas seulement l'un d'eux.
        #
        # Chercher le nom de la condition laissait passer la suppression d'UN
        # des deux tests : l'autre gardait le nom vivant, et la garde se
        # taisait sur exactement la regression qu'elle existe pour attraper.
        non_gardes = 0
        if ecriture is not None:
            lignes = ecriture.splitlines()
            for i, ligne in enumerate(lignes):
                if "lot_recule()" not in ligne:
                    continue
                precedentes = [l.strip() for l in lignes[:i] if l.strip()]
                if not precedentes or "refus_de_taille" not in precedentes[-1]:
                    non_gardes += 1
        if ecriture is not None and (non_gardes or "refus_de_taille" not in ecriture):
            fautes.append(
                "blackbox_storage.rs : le lot recule sur n'importe quel echec. "
                "Une echeance ne dit RIEN de la taille -- elle dit que le "
                "peripherique n'a pas repondu a temps. Sous QEMU, ou treize "
                "cents echeances viennent de l'ordonnancement de l'hote, le lot "
                "tombe a un enregistrement et n'y remonte jamais : le vidage "
                "devient seize fois plus lent et n'aboutit plus. Les deux "
                "machines le disent chacune a leur facon -- la TRIGKEY a "
                "cinquante STALL et zero echeance, QEMU treize cents echeances "
                "et zero STALL."
            )

    # 8. La suite hote confronte le compteur a la formule.
    if test is not None:
        if "ecrases_attendus" not in test:
            fautes.append(
                "test_bobine.rs : le compteur de perte n'est plus confronte a "
                "la formule fermee. Deux facons de compter la meme chose, "
                "dont une seule traverse le chemin chaud : si elles "
                "divergent, c'est le chemin chaud qui a tort."
            )
        if "seconde != 0" not in test:
            fautes.append(
                "test_bobine.rs : le cas a cheval sur la fin de l'anneau n'est "
                "plus exerce. Il ne se produit qu'une fois par tour, et rend "
                "quatre kibioctets de charge utile melangee quand il est faux."
            )

    if fautes:
        for faute in fautes:
            print("FAUTE: %s" % faute)
        return 1
    print(
        "tambour RAM : discipline pure, invariant tenu, chemin chaud sans "
        "peripherique, vidage par lots qui attend son tour."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
