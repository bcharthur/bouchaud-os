#!/usr/bin/env python3
"""Une trame, une soumission (BOUCHAUD_VIRTIO_GPU_TRAME_V1, P10+.3).

Regles :

  1. la soumission par trame est ACTIVE par defaut depuis l'A/B KVM
     CONVAINCANT de b6c99032 (run 38732049308, cite par la statique) :
     `TRAME_VOULUE` vrai, ecrit seulement par `veut_trame`, appele seulement
     par `gpu-trame` ; la CI garde le chemin d'avant (`SCANOUT_TRAME=off`) ;
  2. `presente_trame` ne flushe qu'APRES la reponse du dernier transfert, un
     seul flush par trame, et tient la memoire en `Transfert` autour de
     chaque transfert (meme regle que `presente`) ;
  3. le verrou de la liste n'est jamais tenu pendant une commande :
     `fin_trame` calcule le plan sous le verrou, le relache, puis soumet ;
  4. une region au-dela de la liste rejoint la derniere (aucun pixel perdu) ;
  5. le compositeur ouvre et ferme la trame autour de sa boucle de regions ;
  6. le plan est pur et teste (`tools/display/test_virtio_gpu_trame.rs`) ; la
     CI mesure les pixels sous `gpu-trame on` et l'A/B sous KVM.

Fail-closed ; tests negatifs par mutation.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
FICHIERS = {
    "pur": RACINE / "src/drivers/display/virtio_gpu_trame.rs",
    "test": RACINE / "tools/display/test_virtio_gpu_trame.rs",
    "gpu": RACINE / "src/drivers/display/virtio_gpu.rs",
    "gfx": RACINE / "src/drivers/display/bochs.rs",
    "wm": RACINE / "src/gui/window_manager.rs",
    "shell": RACINE / "src/shell/mod.rs",
    "flux": RACINE / ".github/workflows/os-primitives.yml",
    "analyse": RACINE / "tools/ci/analyse_banc_trame.py",
}
DEFAUT = "static TRAME_VOULUE: AtomicBool = AtomicBool::new(true);"
PREUVE = "CONVAINCANT de\n// b6c99032 (run 38732049308"
ANCIEN = "SCANOUT_TAMPON=direct SCANOUT_TRAME=off BO_QEMU_KVM=1 tools/ci/run_virtio_gpu_scanout.sh"
FLUSH = ("    // Le flush, une fois, apres la reponse du DERNIER transfert.\n    if ok {\n"
         "        if let Some(f) = rogne(plan.flush) {\n"
         "            ok = unsafe { vide(&mut p.file, RESSOURCE_BUREAU, f) } == Ok(RESP_OK_NODATA);")
ARRET = "        if rendu != Ok(RESP_OK_NODATA) {\n            ok = false;\n            break;\n        }"
TENUE = ("        pose_etat_tampon(en_vol);\n"
         "        let rendu = unsafe { transfert(&mut p.file, RESSOURCE_BUREAU, r, decalage) };")
RELACHE = ("        (crate::drivers::virtio_gpu_trame::planifie(&trame.regions[..n], COUT_COMMANDE_PX), n)\n    };")
PLEINE = "        trame.regions[TRAME_REGIONS_MAX - 1] = trame.regions[TRAME_REGIONS_MAX - 1].union(r);"
WM = ("                fb::debut_trame();\n                for region in degats.regions().iter().copied() {",
      "                }\n                fb::fin_trame();")
CI = ("SCANOUT_TRAME=on BO_QEMU_KVM=1 tools/ci/run_virtio_gpu_scanout.sh", "BO_QEMU_KVM=1 tools/ci/run_banc_trame.sh")
TESTS = ("chaque_pixel_endommage_est_transfere_quelle_que_soit_l_entree", "deux_regions_eloignees_restent_separees_mais_un_seul_flush",
         "au_plus_huit_transferts_et_rien_d_oublie")


def corps(texte: str, signature: str) -> str:
    i = texte.find(signature)
    if i < 0:
        return ""
    fin = texte.find("\n}\n", i)
    return texte[i:fin + 3] if fin > 0 else ""


def verifie(t: dict, src: str) -> list:
    fautes = []
    if t["gfx"].count(DEFAUT) != 1 or PREUVE not in t["gfx"]:
        fautes.append("bochs.rs : la soumission par trame n'est plus active par defaut, ou la preuve KVM (run 38732049308) n'est plus citee.")
    if ANCIEN not in t["flux"]:
        fautes.append("CI : le chemin d'avant (deux commandes par region) n'est plus mesure (SCANOUT_TRAME=off).")
    ecrit = t["gfx"].count("TRAME_VOULUE.swap(") + t["gfx"].count("TRAME_VOULUE.store(")
    if ecrit != 1 or "TRAME_VOULUE.swap(oui" not in corps(t["gfx"], "pub fn veut_trame("):
        fautes.append("bochs.rs : la politique est ecrite ailleurs que par veut_trame.")
    appels = [l for l in src.splitlines() if "veut_trame(" in l and "pub fn veut_trame" not in l]
    if len(appels) != 2 or corps(t["shell"], "fn gpu_trame(").count("veut_trame(") != 2:
        fautes.append("la soumission par trame est basculee ailleurs que par la commande gpu-trame.")
    if '"gpu-trame" => gpu_trame(argc, &argv),' not in t["shell"]:
        fautes.append("shell : la commande gpu-trame n'est plus branchee.")
    pt = corps(t["gpu"], "pub fn presente_trame(")
    if FLUSH not in pt or pt.count("vide(&mut p.file") != 1 or ARRET not in pt or pt.find(ARRET) > pt.find(FLUSH):
        fautes.append("presente_trame : le flush n'attend plus la reponse du dernier transfert, ou n'est plus unique.")
    if TENUE not in pt or "tampons::soumet(etat_tampon())" not in pt or "tampons::acheve(en_vol, fin)" not in pt:
        fautes.append("presente_trame : la memoire n'est plus tenue en Transfert autour de chaque transfert.")
    fin = corps(t["gfx"], "pub fn fin_trame(")
    if RELACHE not in fin or fin.find(RELACHE) > fin.find("presente_trame(&plan)"):
        fautes.append("fin_trame : le verrou de la liste est tenu pendant la soumission.")
    if PLEINE not in t["gfx"]:
        fautes.append("bochs.rs : une region au-dela de la liste est perdue.")
    if any(w not in t["wm"] for w in WM):
        fautes.append("window_manager.rs : la boucle des regions n'est plus encadree par la trame.")
    if any(m in t["pur"] for m in ("unsafe", "static ", "asm!")):
        fautes.append("virtio_gpu_trame.rs : le plan n'est plus pur.")
    for nom in TESTS:
        if f"fn {nom}()" not in t["test"]:
            fautes.append(f"test hote absent : {nom}.")
    for c in CI:
        if c not in t["flux"]:
            fautes.append(f"CI : pas KVM absent ({c}).")
    if 'm["ok"] != attendu' not in t["analyse"] or "QUORUM = 4" not in t["analyse"]:
        fautes.append("analyse A/B : critere de surete ou quorum retire.")
    return fautes


def sources() -> str:
    return "\n".join(p.read_text(encoding="utf-8") for p in sorted((RACINE / "src").rglob("*.rs")))


def negatifs(t: dict, src: str) -> list:
    mutations = [
        ("trame desactivee par defaut", "gfx", DEFAUT, DEFAUT.replace("true", "false")),
        ("preuve plus citee", "gfx", PREUVE, "CONVAINCANT"),
        ("ancien chemin plus mesure", "flux", ANCIEN, "true"),
        ("flush sans attendre", "gpu", ARRET, "        let _ = rendu;"),
        ("flush par transfert", "gpu", "        pixels += u64::from(r.largeur) * u64::from(r.hauteur);",
         "        pixels += u64::from(r.largeur) * u64::from(r.hauteur);\n        let _ = unsafe { vide(&mut p.file, RESSOURCE_BUREAU, r) };"),
        ("transfert sans tenue", "gpu", TENUE, "        let rendu = unsafe { transfert(&mut p.file, RESSOURCE_BUREAU, r, decalage) };"),
        ("region perdue", "gfx", PLEINE, "        let _ = r;"),
        ("compositeur sans trame", "wm", WM[0], "                for region in degats.regions().iter().copied() {"),
        ("plan impur", "pur", "pub fn planifie(", "pub unsafe fn planifie("),
        ("test de couverture retire", "test", "fn chaque_pixel_endommage_est_transfere_quelle_que_soit_l_entree()", "fn x()"),
        ("pas KVM retire", "flux", CI[1], "true"),
        ("trame perdue toleree", "analyse", 'm["ok"] != attendu', "False"),
        ("commande debranchee", "shell", '"gpu-trame" => gpu_trame(argc, &argv),', ""),
    ]
    echecs = []
    for nom, cle, avant, apres in mutations:
        if avant not in t[cle]:
            echecs.append(f"mutation inapplicable : {nom}")
            continue
        m = dict(t)
        m[cle] = t[cle].replace(avant, apres, 1)
        s = src.replace(t[cle], m[cle]) if cle in ("gpu", "gfx", "shell", "pur", "wm") else src
        if not verifie(m, s):
            echecs.append(f"mutation non detectee : {nom}")
    s = src + "\nfn x() { crate::drivers::gfx::veut_trame(true); }\n"
    if not verifie(t, s):
        echecs.append("mutation non detectee : bascule hors du shell")
    return echecs


def main() -> int:
    t = {k: p.read_text(encoding="utf-8") for k, p in FICHIERS.items()}
    src = sources()
    fautes = verifie(t, src) + negatifs(t, src)
    if fautes:
        for f in fautes:
            print(f"verifie-virtio-gpu-trame: {f}")
        return 1
    print("verifie-virtio-gpu-trame: OK (on par defaut (A/B KVM 38732049308), ancien chemin mesure, un flush apres le dernier transfert, memoire tenue, "
          "verrou relache avant la soumission, aucune region perdue, compositeur encadre, plan pur, CI KVM ; 15 negatifs)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
