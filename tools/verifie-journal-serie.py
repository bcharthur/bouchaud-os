#!/usr/bin/env python3
"""Garde-fou : le journal physique ne doit pas etre perdu avant d'etre ecrit.

# Le defaut, mesure

Le tambour de trace faisait soixante-quatre kilooctets. C'est moins qu'UN
demarrage : le scenario `run_os_primitives` produit 75 852 octets de journal
sans bureau ni navigateur, et un seul releve periodique du gestionnaire de
fenetres en fait 18 426.

L'enregistreur de vol, lui, ne sait poser un octet qu'une fois la cle USB
enumeree -- donc APRES la partie du demarrage qu'on cherche le plus souvent a
relire. Et il ne posait qu'un enregistrement de 4 032 octets par fenetre de
250 ms, soit seize kilooctets par seconde, quoi qu'il y ait a poser. Le retard
ne se resorbait jamais.

Troisieme defaut, sur la machine physique cette fois : `init()` affirmait
`INITIALISED = true` sans demander a COM1 s'il existait. Une TRIGKEY n'expose
pas de Super I/O. Chaque octet de journal partait donc vers un port que
personne ne decode -- une attente de THRE puis seize `outb` par lot de seize
octets. La verbosite devenait une taxe sur la machine qu'elle devait decrire.

# Ce qui est verifie

1. Le tambour tient au moins un mebioctet.
2. `write_lot` capture dans le tambour AVANT de regarder si COM1 existe.
3. `write_lot` renonce au port quand la sonde ne dit pas « present ».
4. `init()` interroge le materiel au lieu de l'affirmer.
5. Le verdict de la sonde est une fonction PURE, verifiable sur l'hote.
6. Le drain serie est tire du retard reel, pas d'une constante.
7. L'echantillon porte la perte cumulee : un journal troue doit se voir.
8. La marque de depart dit ce que le demarrage avait deja produit.
9. L'echantillon par coeur suit `schedulable_cpus()`, pas quatre.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
UART = RACINE / "src/drivers/serial/uart16550.rs"
SONDE = RACINE / "src/drivers/serial/sonde.rs"
BLACKBOX = RACINE / "src/kernel/debug/blackbox.rs"

MEBIOCTET = 1024 * 1024


def corps(source, entete):
    """Le corps d'une fonction, accolades comprises."""
    debut = source.find(entete)
    if debut < 0:
        return None
    i = source.find("{", debut)
    if i < 0:
        return None
    profondeur = 0
    for j in range(i, len(source)):
        if source[j] == "{":
            profondeur += 1
        elif source[j] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[i:j + 1]
    return None


def sans_commentaires(source):
    """Le code seul : un motif cite dans un commentaire ne prouve rien."""
    return "\n".join(
        ligne for ligne in source.splitlines()
        if not ligne.lstrip().startswith("//")
    )


def main():
    fautes = []
    for chemin in (UART, SONDE, BLACKBOX):
        if not chemin.exists():
            print("  - fichier absent : %s" % chemin)
            return 1

    uart = sans_commentaires(UART.read_text(encoding="utf-8"))
    sonde = SONDE.read_text(encoding="utf-8")
    blackbox = sans_commentaires(BLACKBOX.read_text(encoding="utf-8"))

    # --- 1. La taille du tambour -------------------------------------------
    taille = re.search(r"const TRACE_BYTES: usize = ([0-9_ *]+);", uart)
    if not taille:
        fautes.append(
            "uart16550.rs : `TRACE_BYTES` introuvable ; la capacite du tambour "
            "n'est plus une constante lisible."
        )
    else:
        try:
            octets = eval(taille.group(1).replace("_", ""), {"__builtins__": {}})
        except Exception:
            octets = 0
        if octets < MEBIOCTET:
            fautes.append(
                "uart16550.rs : le tambour fait %d octets. Un seul demarrage en "
                "produit plus de 75 000 et un releve periodique 18 426 : sous un "
                "mebioctet, la partie du demarrage anterieure a l'enumeration USB "
                "est perdue avant que l'enregistreur sache l'ecrire." % octets
            )

    # --- 2 et 3. Le tambour d'abord, le port ensuite ------------------------
    lot = corps(uart, "fn write_lot(")
    if lot is None:
        fautes.append("uart16550.rs : `write_lot` introuvable.")
    else:
        pose_tambour = lot.find("trace_capture(")
        renonce = lot.find("presence_com1()")
        if pose_tambour < 0:
            fautes.append(
                "uart16550.rs : `write_lot` n'ecrit plus dans le tambour. C'est "
                "le tambour, et non COM1, que l'enregistreur pose sur la cle USB."
            )
        if renonce < 0:
            fautes.append(
                "uart16550.rs : `write_lot` ecrit sur COM1 sans demander si le "
                "port existe. Sur une machine sans Super I/O, chaque octet de "
                "journal coute une attente de THRE pour rien."
            )
        elif pose_tambour >= 0 and renonce < pose_tambour:
            fautes.append(
                "uart16550.rs : `write_lot` consulte la sonde AVANT d'ecrire dans "
                "le tambour. Un port absent ferait alors disparaitre le journal "
                "physique, qui est precisement ce qu'on cherche a garder."
            )
        elif not re.search(r"presence_com1\(\)\.ecrire\(\)\s*\{\s*return", lot):
            fautes.append(
                "uart16550.rs : `write_lot` consulte la sonde sans en tirer de "
                "consequence ; le port est ecrit quel que soit le verdict."
            )

    # --- 4. init() interroge au lieu d'affirmer -----------------------------
    init = corps(uart, "pub fn init(")
    if init is None:
        fautes.append("uart16550.rs : `init` introuvable.")
    elif "sonde_com1()" not in init or "PRESENCE.store" not in init:
        fautes.append(
            "uart16550.rs : `init` ne sonde plus COM1. Poser `INITIALISED = true` "
            "sans demander au materiel, c'est ce qui faisait payer le journal a "
            "une machine qui n'a pas de port serie."
        )

    # --- 5. Le verdict reste pur -------------------------------------------
    if "fn verdict(" not in sonde:
        fautes.append(
            "sonde.rs : `verdict` a disparu ; la decision de la sonde n'est plus "
            "verifiable sur l'hote."
        )
    code_sonde = sans_commentaires(sonde)
    for interdit, pourquoi in (
        ("use crate::", "il depend du noyau"),
        ("unsafe", "il touche au materiel"),
    ):
        if interdit in code_sonde:
            fautes.append(
                "sonde.rs : `%s` est apparu ; %s, donc il ne se compile plus seul "
                "sur l'hote et le test de `tools/platform/test_sonde_uart.rs` "
                "cesserait de couvrir la decision." % (interdit, pourquoi)
            )
    # Le cas du bus flottant doit etre TRAITE, pas seulement nomme : c'est son
    # retour anticipe, avant la comparaison au motif, qui rend le verdict
    # insensible au motif choisi.
    decision = corps(code_sonde, "pub fn verdict(")
    if decision is None:
        fautes.append("sonde.rs : le corps de `verdict` est illisible.")
    else:
        flottant = re.search(
            r"if\s+lsr\s*==\s*0xFF\s*&&\s*iir\s*==\s*0xFF\s*\{\s*return\s+"
            r"Presence::BusFlottant\s*;",
            decision,
        )
        motif = decision.find("== motif")
        if not flottant:
            fautes.append(
                "sonde.rs : `verdict` ne renonce plus des que tous les registres "
                "rendent 0xFF. Un port non decode serait lu comme un UART present "
                "des que le motif vaut 0xFF, et une machine sans Super I/O "
                "repaierait le journal qu'elle ne peut pas emettre."
            )
        elif motif >= 0 and flottant.start() > motif:
            fautes.append(
                "sonde.rs : `verdict` compare au motif AVANT d'ecarter le bus "
                "flottant. L'ordre est le fond de la decision, pas un detail."
            )

    # --- 6. Le drain suit le retard ----------------------------------------
    drain = corps(blackbox, "fn flush_serial(")
    if drain is None:
        fautes.append("blackbox.rs : `flush_serial` introuvable.")
    else:
        if "div_ceil(PAYLOAD_MAX)" not in drain:
            fautes.append(
                "blackbox.rs : `flush_serial` ne calcule plus le nombre "
                "d'enregistrements a partir du retard. Un plafond constant fixe "
                "le debit du journal a seize kilooctets par seconde, alors qu'un "
                "seul releve periodique en fait dix-huit mille."
            )
        if "RECORDS_MAX_PAR_FENETRE" not in drain:
            fautes.append(
                "blackbox.rs : `flush_serial` n'a plus de borne par fenetre. Un "
                "drain non borne transformerait un retard en figement de la "
                "fenetre de scrutation."
            )
        if "SERIAL_PERDUS" not in drain:
            fautes.append(
                "blackbox.rs : `flush_serial` ne compte plus ce qu'il a perdu. "
                "Un journal troue se lirait alors comme un journal complet."
            )

    # --- 7. La perte figure dans l'echantillon ------------------------------
    echantillon = corps(blackbox, "fn sample(")
    if echantillon is None:
        fautes.append("blackbox.rs : `sample` introuvable.")
    else:
        for champ in ("serial_perdus=", "serial_retard=", "com1="):
            if champ not in echantillon:
                fautes.append(
                    "blackbox.rs : `sample` ne publie plus `%s`. La sante du "
                    "journal doit se lire dans le journal lui-meme, sinon on "
                    "raisonne sur des traces dont on ignore si elles sont "
                    "completes." % champ
                )

    # --- 8. La marque de depart situe le demarrage --------------------------
    scrutation = corps(blackbox, "pub fn poll(")
    if scrutation is None:
        fautes.append("blackbox.rs : `poll` introuvable.")
    elif "arriere=" not in scrutation or "capacite=" not in scrutation:
        fautes.append(
            "blackbox.rs : la marque START ne dit plus ce que le demarrage avait "
            "deja produit. C'est le seul endroit ou l'on peut constater qu'aucune "
            "ligne anterieure a l'enumeration USB n'a ete perdue."
        )

    # --- 9. Tous les coeurs, pas quatre ------------------------------------
    par_cpu = corps(blackbox, "fn echantillon_par_cpu(")
    if par_cpu is None:
        fautes.append(
            "blackbox.rs : `echantillon_par_cpu` a disparu. L'echantillon "
            "decrivait quatre coeurs sur les seize de la machine de reference, "
            "et un figement sur les douze autres ressemblait a une machine saine."
        )
    elif "schedulable_cpus()" not in par_cpu:
        fautes.append(
            "blackbox.rs : `echantillon_par_cpu` ne suit plus le nombre de coeurs "
            "ordonnancables ; un nombre fige reintroduit exactement l'angle mort "
            "qu'il devait supprimer."
        )

    if fautes:
        print("journal serie : %d probleme(s)" % len(fautes))
        for faute in fautes:
            print("\n  - %s" % faute)
        return 1

    print(
        "journal serie : tambour d'un mebioctet, COM1 sonde au lieu d'etre "
        "suppose, drain tire du retard, perte et retard publies, tous les coeurs "
        "echantillonnes"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
