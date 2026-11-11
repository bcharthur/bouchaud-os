#!/usr/bin/env python3
"""BOUCHAUD_GPU_REPLI_SURVIT_A_LA_TRAME_V1 + BOUCHAUD_GPU_TEXTURE_SUIT_L_AVEUGLE_V1.

Deux defauts de la composition GPU des fenetres (revue de la PR #287), dans
la boucle du bureau noyau -- sans banc hote possible, donc gardes avec
temoins negatifs :

  1. quand le GPU refuse une trame, `ferme_trame` le DIT (retour `bool`) et
     la boucle REINSTALLE le degat plein ecran et `sale` APRES son
     `degats.efface()` de fin de trame : sinon la repeinture CPU promise est
     effacee avant d'etre peinte et un client immobile reste perime ;
  2. un client recompose « a l'aveugle » (aucune notification de trame), ou
     qui sort du silence, marque toute sa texture GPU (`degat_gpu`) et
     `gpu_sale` : sinon le GPU recopie indefiniment une texture figee.
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
WM = "src/gui/window_manager.rs"


def verifie(racine: Path) -> list:
    t = (racine / WM).read_text(encoding="utf-8")
    fautes = []
    i = t.find("fn ferme_trame(")
    corps = t[i:t.find("\nfn ", i + 1)]
    if i < 0 or ") -> bool {" not in corps or "\n    repli\n}" not in corps:
        fautes.append("ferme_trame ne rend plus le repli CPU a l'appelant")
    if "repli_cpu_du = ferme_trame(" not in t:
        fautes.append("la boucle ignore le repli rendu par ferme_trame")
    m = re.search(r"sale = false;\n\s*degats\.efface\(\);\n(.*?)\n        \}", t, re.S)
    if not m or not re.search(r"if core::mem::take\(&mut repli_cpu_du\) \{[^}]*degats\.tout\(\);[^}]*sale = true;",
                              m.group(1), re.S):
        fautes.append("le degat du repli CPU n'est pas reinstalle apres degats.efface()")
    j = t.find("fn pompe_clients(")
    pompe = t[j:t.find("\nfn ", j + 1)]
    for garde, nom in (("if client.verifie_silence() {", "sortie de silence"),
                       ("if recompose_aveugle && client.recompose_a_l_aveugle() && visible {", "recomposition a l'aveugle")):
        k = pompe.find(garde)
        bloc = pompe[k:pompe.find("\n            }", k)] if k >= 0 else ""
        if "client.degat_gpu = client.surface_entiere();" not in bloc or "gpu_sale = true;" not in bloc:
            fautes.append(f"{nom} : la texture GPU du client n'est pas remontee")
    return fautes


def temoins():
    mutations = [
        ("repli ignore", "repli_cpu_du = ferme_trame(", "let _ = ferme_trame("),
        ("repli efface", "                degats.tout();\n                sale = true;\n            }\n        }",
         "            }\n        }"),
        ("aveugle sans texture", "                // doit etre remontee, sinon le GPU recopie une image figee.\n"
         "                client.degat_gpu = client.surface_entiere();\n",
         "                // doit etre remontee, sinon le GPU recopie une image figee.\n"),
        ("silence sans texture", "                // BOUCHAUD_GPU_TEXTURE_SUIT_L_AVEUGLE_V1 (revue de la PR #287)\n"
         "                client.degat_gpu = client.surface_entiere();\n                gpu_sale = true;\n",
         "                // BOUCHAUD_GPU_TEXTURE_SUIT_L_AVEUGLE_V1 (revue de la PR #287)\n"),
        ("ferme_trame ne rend rien", "\n    repli\n}", "\n    let _ = repli;\n}"),
    ]
    texte = (RACINE / WM).read_text(encoding="utf-8")
    for nom, a, b in mutations:
        if texte.count(a) != 1:
            return f"temoin {nom} : motif introuvable ({texte.count(a)})"
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / WM).parent.mkdir(parents=True)
            (Path(d) / WM).write_text(texte.replace(a, b, 1), encoding="utf-8")
            if not verifie(Path(d)):
                return f"temoin negatif {nom} ne rougit pas"
    return None


def main():
    fautes = verifie(RACINE)
    for f in fautes:
        print("repli GPU / aveugle :", f)
    if fautes:
        return 1
    t = temoins()
    if t:
        print("repli GPU / aveugle :", t)
        return 1
    print("REPLI_GPU_ET_AVEUGLE_OK negatifs=5")
    return 0


if __name__ == "__main__":
    sys.exit(main())
