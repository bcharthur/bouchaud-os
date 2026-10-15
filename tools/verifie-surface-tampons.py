#!/usr/bin/env python3
"""Le compositeur ne lit jamais un tampon qu'un client conforme peut ecrire
(BOUCHAUD_GUI_SURFACE_DEUX_TAMPONS_V1, P10+.3c).

Regles :

  1. l'offre de deux tampons est DESACTIVEE par defaut
     (`SURFACE_DOUBLE_OFFERTE` faux, ecrit seulement par
     `offre_surface_double`, appele seulement par `gui-surface`) ; de meme
     l'instrument de dechirure (`MESURE_DECHIRURE`) ;
  2. le contrat se fixe a la poignee de main, par la regle pure :
     `Tampons::negocie(self.surface.tampons, capacites)` -- deux tampons
     seulement si offerts ET annonces ;
  3. `FrameReady` passe par `trame_prete` ; un tampon hors surface est
     refuse AVANT que son degat ne serve ;
  4. les liberations partent apres le traitement des messages, et un tampon
     n'est marque libere qu'une fois sa `Liberation` reellement envoyee (un
     canal plein la retarde, ne la perd pas) ;
  5. la composition lit la FACE (`copie_ligne_de(client.face(), ...)`), et
     nulle part la surface autrement ;
  6. la regle est pure et testee (`tools/gui/test_surface_tampons.rs`) ; la
     CI joue le banc de dechirure sous TCG et KVM, criteres declares.

Fail-closed ; tests negatifs par mutation.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
FICHIERS = {
    "pur": RACINE / "src/gui/surface_tampons.rs",
    "test": RACINE / "tools/gui/test_surface_tampons.rs",
    "client": RACINE / "src/gui/client.rs",
    "widgets": RACINE / "src/gui/widgets.rs",
    "surface": RACINE / "src/gui/surface.rs",
    "shell": RACINE / "src/shell/mod.rs",
    "flux": RACINE / ".github/workflows/os-primitives.yml",
    "analyse": RACINE / "tools/ci/analyse_banc_dechirure.py",
}
OFFRE = "static SURFACE_DOUBLE_OFFERTE: AtomicBool = AtomicBool::new(false);"
MESURE = "static MESURE_DECHIRURE: AtomicBool = AtomicBool::new(false);"
NEGOCIE = "self.tampons = Tampons::negocie(self.surface.tampons, capacites);"
HORS = ("                            Issue::HorsBornes => {\n"
        "                                self.bilan.hors_bornes += 1;\n"
        "                                return false;\n                            }")
ENVOI = ("            if !self.envoie(Genre::Liberation, &charge) {\n"
         "                self.bilan.liberations_retardees += 1;\n                break;\n            }\n"
         "            self.tampons.liberation_envoyee(t);")
APRES = "        self.envoie_liberations();\n        trame_prete\n    }"
FACE = "surface.copie_ligne_de(client.face(), y - zy, colonne, pris, destination);"
CI = ("tools/ci/run_banc_dechirure.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin",
      "BO_QEMU_KVM=1 tools/ci/run_banc_dechirure.sh")
TESTS = ("un_client_conforme_n_ecrit_jamais_dans_la_face", "le_client_retrouve_toujours_un_tampon_libre",
         "une_liberation_qui_n_a_pas_pu_partir_reste_due", "la_negociation_exige_l_offre_et_la_capacite")


def corps(texte: str, signature: str) -> str:
    i = texte.find(signature)
    if i < 0:
        return ""
    fin = texte.find("\n    }\n", i)
    fin2 = texte.find("\n}\n", i)
    fins = [f for f in (fin, fin2) if f > 0]
    return texte[i:min(fins) + 6] if fins else ""


def verifie(t: dict, src: str) -> list:
    fautes = []
    if t["client"].count(OFFRE) != 1 or t["client"].count(MESURE) != 1:
        fautes.append("client.rs : l'offre de deux tampons ou l'instrument de dechirure n'est plus desactive par defaut.")
    for statique, fonction in (("SURFACE_DOUBLE_OFFERTE", "offre_surface_double"), ("MESURE_DECHIRURE", "mesure_dechirure")):
        ecrit = t["client"].count(f"{statique}.swap(") + t["client"].count(f"{statique}.store(")
        if ecrit != 1 or f"{statique}.swap(oui" not in corps(t["client"], f"pub fn {fonction}("):
            fautes.append(f"client.rs : {statique} est ecrite ailleurs que par {fonction}.")
        appels = [l for l in src.splitlines() if f"{fonction}(" in l and f"pub fn {fonction}" not in l]
        if len(appels) != (2 if fonction == "offre_surface_double" else 1) \
                or any("crate::gui::client::" not in l for l in appels):
            fautes.append(f"{fonction} est appelee ailleurs que par la commande gui-surface.")
    if '"gui-surface" => gui_surface(argc, &argv),' not in t["shell"]:
        fautes.append("shell : la commande gui-surface n'est plus branchee.")
    if t["client"].count(NEGOCIE) != 1 or NEGOCIE not in corps(t["client"], "    fn traite("):
        fautes.append("client.rs : le contrat n'est plus fixe a la poignee de main par Tampons::negocie.")
    traite = t["client"]
    if HORS not in traite or traite.find(HORS) > traite.find("let degat = match degat {"):
        fautes.append("client.rs : une trame hors surface n'est plus refusee avant que son degat serve.")
    if ENVOI not in t["client"]:
        fautes.append("client.rs : un tampon est marque libere sans que sa Liberation soit partie.")
    if APRES not in t["client"]:
        fautes.append("client.rs : les liberations ne partent plus apres le traitement des messages.")
    if t["widgets"].count(FACE) != 1:
        fautes.append("widgets.rs : la composition ne lit plus la face de la surface.")
    lectures = [l for l in src.splitlines() if ".copie_ligne(" in l or ".copie_ligne_de(" in l]
    lectures = [l for l in lectures if "pub fn " not in l and "self.copie_ligne_de(0," not in l]
    if len(lectures) != 1:
        fautes.append(f"la surface est lue ailleurs que par la composition de la face ({len(lectures)} sites).")
    if any(m in t["pur"] for m in ("unsafe", "static mut", "asm!")):
        fautes.append("surface_tampons.rs : la regle n'est plus pure.")
    if "tampon >= self.tampons" not in corps(t["surface"], "pub fn copie_ligne_de("):
        fautes.append("surface.rs : une lecture hors des tampons n'est plus refusee.")
    for nom in TESTS:
        if f"fn {nom}()" not in t["test"]:
            fautes.append(f"test hote absent : {nom}.")
    for c in CI:
        if c not in t["flux"]:
            fautes.append(f"CI : banc absent ({c}).")
    for jalon in ('bi["mixtes"] != 0', "QUORUM = 4", "temoin = mixtes_v1 > 0", "LECTURES_MIN = 20"):
        if jalon not in t["analyse"]:
            fautes.append(f"analyse du banc : critere retire ({jalon}).")
    return fautes


def sources() -> str:
    return "\n".join(p.read_text(encoding="utf-8") for p in sorted((RACINE / "src").rglob("*.rs")))


def negatifs(t: dict, src: str) -> list:
    mutations = [
        ("offre active par defaut", "client", OFFRE, OFFRE.replace("false", "true")),
        ("mesure active par defaut", "client", MESURE, MESURE.replace("false", "true")),
        ("contrat impose sans capacite", "client", NEGOCIE, "self.tampons = Tampons::double();"),
        ("trame hors surface acceptee", "client", "                                return false;\n                            }\n"
         "                            Issue::EcritureSurFace", "                            }\n                            Issue::EcritureSurFace"),
        ("liberation marquee sans envoi", "client", "                self.bilan.liberations_retardees += 1;\n                break;\n",
         "                self.bilan.liberations_retardees += 1;\n"),
        ("liberations jamais envoyees", "client", "        self.envoie_liberations();\n        trame_prete", "        trame_prete"),
        ("composition hors face", "widgets", FACE, "surface.copie_ligne_de(0, y - zy, colonne, pris, destination);"),
        ("regle impure", "pur", "pub fn trame_prete(", "pub unsafe fn trame_prete("),
        ("lecture hors tampons", "surface", "tampon >= self.tampons || ", ""),
        ("test de surete retire", "test", "fn un_client_conforme_n_ecrit_jamais_dans_la_face()", "fn x()"),
        ("banc KVM retire", "flux", CI[1], "true"),
        ("dechirure toleree en v2", "analyse", 'bi["mixtes"] != 0', "False"),
        ("temoin retire", "analyse", "temoin = mixtes_v1 > 0", "temoin = True"),
        ("commande debranchee", "shell", '"gui-surface" => gui_surface(argc, &argv),', ""),
    ]
    echecs = []
    for nom, cle, avant, apres in mutations:
        if avant not in t[cle]:
            echecs.append(f"mutation inapplicable : {nom}")
            continue
        m = dict(t)
        m[cle] = t[cle].replace(avant, apres, 1)
        s = src.replace(t[cle], m[cle]) if cle in ("client", "widgets", "surface", "shell", "pur") else src
        if not verifie(m, s):
            echecs.append(f"mutation non detectee : {nom}")
    s = src + "\nfn x() { crate::gui::client::offre_surface_double(true); }\n"
    if not verifie(t, s):
        echecs.append("mutation non detectee : offre basculee hors du shell")
    s = src + "\nfn y(s: &Surface, d: &mut [u32]) { s.copie_ligne(0, 0, 1, d); }\n"
    if not verifie(t, s):
        echecs.append("mutation non detectee : surface lue hors de la composition")
    return echecs


def main() -> int:
    t = {k: p.read_text(encoding="utf-8") for k, p in FICHIERS.items()}
    src = sources()
    fautes = verifie(t, src) + negatifs(t, src)
    if fautes:
        for f in fautes:
            print(f"verifie-surface-tampons: {f}")
        return 1
    print("verifie-surface-tampons: OK (offre et mesure off par defaut, contrat negocie a la poignee de main, "
          "hors surface refuse, liberations jamais perdues, composition sur la face, regle pure, banc TCG+KVM ; 16 negatifs)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
