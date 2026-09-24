#!/usr/bin/env python3
"""L'extracteur distingue COMPLETE, PARTIEL_CHECKPOINT et COUPURE.

BOUCHAUD_C72_CHECKPOINT_FAIL_SAFE

Au test physique du Trigkey (image d131f2a), l'extinction a echoue et TOUTE la
session a ete perdue -- y compris les journaux qui expliquaient le reseau. Le
checkpoint existe pour qu'au pire on ne perde que la derniere fenetre.

Encore faut-il que l'extracteur le DISE. Une archive sans marque de FIN mais
avec un checkpoint valide n'est pas « complete » : elle est partielle jusqu'au
checkpoint N. Les confondre, c'est soit jeter des preuves utilisables, soit
croire qu'on a tout alors qu'il manque la fin.

Trois etats, et les deux confusions interdites sont testees en negatif.
"""
import importlib.util
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[2]
EXTRACTEUR = RACINE / "tools/reference/extract-blackbox.py"

FIN = "BOUCHAUD_TRIGKEY_BLACKBOX_V3 FIN raison=arret boot_id=7 ts_ns=1 draine=1\n"
CP1 = ("BOUCHAUD_TRIGKEY_BLACKBOX_V3 CHECKPOINT raison=periodique boot_id=7 "
       "seq=1 ts_ns=5 dernier_confirme=120\n")
CP2 = ("BOUCHAUD_TRIGKEY_BLACKBOX_V3 CHECKPOINT raison=manuel boot_id=7 "
       "seq=2 ts_ns=9 dernier_confirme=304\n")
RECS = [{"seq": n} for n in (1, 2, 3, 5)]   # un trou volontaire en 4


def charge():
    spec = importlib.util.spec_from_file_location("eb", EXTRACTEUR)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main() -> int:
    m = charge()
    cas = [
        ("session complete",    [FIN, CP1], "COMPLETE",           1, 1,    120),
        ("checkpoint sans FIN", [CP1, CP2], "PARTIEL_CHECKPOINT", 2, 2,    304),
        ("coupure seche",       [],         "COUPURE",            0, None, None),
        ("un seul checkpoint",  [CP1],      "PARTIEL_CHECKPOINT", 1, 1,    120),
    ]
    erreurs = []
    for nom, marques, attendu, n_cp, seq, confirme in cas:
        v = m.verdict_completude(marques, RECS)
        for cle, obtenu, voulu in (
            ("completude", v["completude"], attendu),
            ("checkpoints", v["checkpoints"], n_cp),
            ("dernier_checkpoint_seq", v["dernier_checkpoint_seq"], seq),
            ("dernier_checkpoint_confirme", v["dernier_checkpoint_confirme"], confirme),
            ("trous", v["trous"], 1),
        ):
            if obtenu != voulu:
                erreurs.append(f"{nom}: {cle}={obtenu} attendu {voulu}")

    # Les deux confusions qui coutent le plus cher.
    if m.verdict_completude([CP1], RECS)["completude"] == "COMPLETE":
        erreurs.append("negatif: un checkpoint seul passe pour une session complete")
    if m.verdict_completude([], RECS)["fin_presente"]:
        erreurs.append("negatif: une coupure seche pretend avoir une marque FIN")

    if erreurs:
        for e in erreurs:
            print(f"EXTRACT_VERDICT_FAIL {e}")
        print(f"EXTRACT_VERDICT echec erreurs={len(erreurs)}")
        return 1
    print("EXTRACT_VERDICT ok cas=4 negatifs=2")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
