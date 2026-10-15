#!/usr/bin/env python3
"""La memoire du scanout virtio-gpu n'est jamais ecrite pendant que l'hote
peut la lire, et chaque copie d'une trame est comptee
(BOUCHAUD_VIRTIO_GPU_TAMPONS_V1, P10+.2).

Regles :

  1. la composition directe est DESACTIVEE par defaut :
     `TAMPON_DIRECT_VOULU` est un `AtomicBool::new(false)`, ecrit seulement
     par `veut_tampon_direct`, appele seulement par la commande `gpu-tampon` ;
     toute entree en mode graphique repart en copie avant de decider ;
  2. `presente` passe la memoire en `Transfert` (`tampons::soumet`) AVANT le
     `TRANSFER_TO_HOST_2D` et la rend (`tampons::acheve`, sans reponse ->
     `Inconnu`) APRES sa reponse -- et refuse de presenter si `soumet` refuse ;
  3. en composition directe, tout acces au tampon du bureau pendant un
     transfert est COMPTE (`ACCES_EN_TRANSFERT`) ;
  4. l'abandon du scanout quitte sa memoire (`quitte_memoire_scanout` : plus
     d'ecriture, repli sur un tampon arriere) ;
  5. `present` et `present_rect` comptent les octets copies et les octets
     presentes sans copie ;
  6. la regle est pure et testee (`tools/display/test_virtio_gpu_tampons.rs`) ;
     la CI mesure les pixels en composition directe et l'A/B sous KVM.

Fail-closed ; tests negatifs par mutation.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
FICHIERS = {
    "pur": RACINE / "src/drivers/display/virtio_gpu_tampons.rs",
    "test": RACINE / "tools/display/test_virtio_gpu_tampons.rs",
    "gpu": RACINE / "src/drivers/display/virtio_gpu.rs",
    "gfx": RACINE / "src/drivers/display/bochs.rs",
    "shell": RACINE / "src/shell/mod.rs",
    "flux": RACINE / ".github/workflows/os-primitives.yml",
    "analyse": RACINE / "tools/ci/analyse_banc_tampon.py",
}
DEFAUT = "static TAMPON_DIRECT_VOULU: AtomicBool = AtomicBool::new(false);"
REPART = "fn enter_verrouille() {\n    TAMPON_DIRECT.store(false, Ordering::Release);"
SOUMET = ("    let Some(en_vol) = tampons::soumet(etat_tampon()) else {\n"
          "        PRESENTS_ECHEC.fetch_add(1, Ordering::Relaxed);\n        return false;\n    };\n"
          "    pose_etat_tampon(en_vol);")
TRANSFERT = ("    let rendu = unsafe { transfert(&mut p.file, RESSOURCE_BUREAU, r, decalage) };\n"
             "    let fin = if dma::issue(rendu) == dma::Issue::SansReponse { tampons::Fin::SansReponse } "
             "else { tampons::Fin::Achevee };\n"
             "    pose_etat_tampon(tampons::acheve(en_vol, fin));")
ACCES = ("        if !crate::drivers::virtio_gpu::tampon_ecrivable() {\n"
         "            ACCES_EN_TRANSFERT.fetch_add(1, Ordering::Relaxed);\n        }")
ABANDON = "        VIRTIO_SCANOUT.store(false, Ordering::Release);\n        quitte_memoire_scanout();"
QUITTE = "        LFB = core::ptr::null_mut();\n    }\n}"
CI = ("SCANOUT_TAMPON=direct BO_QEMU_KVM=1 tools/ci/run_virtio_gpu_scanout.sh",
      "BO_QEMU_KVM=1 tools/ci/run_banc_tampon.sh")
TESTS = ("aucune_ecriture_pendant_un_transfert", "un_transfert_sans_reponse_rend_le_tampon_inconnu_pour_toujours",
         "les_copies_d_une_trame_sont_comptees", "aucun_ecrivain_ne_passe_pendant_un_transfert_quelle_que_soit_la_fin")


def corps(texte: str, signature: str) -> str:
    i = texte.find(signature)
    if i < 0:
        return ""
    fin = texte.find("\n}\n", i)
    return texte[i:fin + 3] if fin > 0 else ""


def verifie(t: dict, src: str) -> list:
    fautes = []
    if t["gfx"].count(DEFAUT) != 1:
        fautes.append("bochs.rs : la composition directe n'est plus desactivee par defaut.")
    ecrit = t["gfx"].count("TAMPON_DIRECT_VOULU.swap(") + t["gfx"].count("TAMPON_DIRECT_VOULU.store(")
    if ecrit != 1 or "TAMPON_DIRECT_VOULU.swap(oui" not in corps(t["gfx"], "pub fn veut_tampon_direct("):
        fautes.append("bochs.rs : la politique est ecrite ailleurs que par veut_tampon_direct.")
    appels = [l for l in src.splitlines() if "veut_tampon_direct(" in l and "pub fn veut_tampon_direct" not in l]
    if len(appels) != 2 or corps(t["shell"], "fn gpu_tampon(").count("veut_tampon_direct(") != 2:
        fautes.append("la composition directe est basculee ailleurs que par la commande gpu-tampon.")
    if '"gpu-tampon" => gpu_tampon(argc, &argv),' not in t["shell"]:
        fautes.append("shell : la commande gpu-tampon n'est plus branchee.")
    if t["gfx"].count(REPART) != 1:
        fautes.append("bochs.rs : une entree en mode graphique ne repart plus en copie.")
    presente = corps(t["gpu"], "pub fn presente(")
    if SOUMET not in presente or TRANSFERT not in presente \
            or presente.find(SOUMET) > presente.find("transfert(&mut p.file"):
        fautes.append("virtio_gpu.rs : la memoire n'est plus tenue en Transfert autour du TRANSFER_TO_HOST_2D.")
    if ACCES not in corps(t["gfx"], "fn back() -> &'static mut [u32] {"):
        fautes.append("bochs.rs : un acces au tampon pendant un transfert n'est plus compte.")
    # Chaque abandon du scanout par le bureau (presentation ou fin de trame)
    # en quitte la memoire.
    abandons = t["gfx"].count("\n        VIRTIO_SCANOUT.store(false, Ordering::Release);\n")
    if abandons < 1 or t["gfx"].count(ABANDON) != abandons \
            or ABANDON not in corps(t["gfx"], "fn presente_virtio(") \
            or QUITTE not in corps(t["gfx"], "fn quitte_memoire_scanout("):
        fautes.append("bochs.rs : l'abandon du scanout n'en quitte plus la memoire.")
    for f in ("pub fn present() {", "pub fn present_rect("):
        c = corps(t["gfx"], f)
        if "OCTETS_COPIES.fetch_add(" not in c or "OCTETS_DIRECTS.fetch_add(" not in c:
            fautes.append(f"bochs.rs : {f.split('(')[0]} ne compte plus ses copies.")
    if any(m in t["pur"] for m in ("unsafe", "static ", "asm!")):
        fautes.append("virtio_gpu_tampons.rs : la regle n'est plus pure.")
    for nom in TESTS:
        if f"fn {nom}()" not in t["test"]:
            fautes.append(f"test hote absent : {nom}.")
    for c in CI:
        if c not in t["flux"]:
            fautes.append(f"CI : pas KVM absent ({c}).")
    for jalon in ('d["copies"] != 0', 'd["acces"] != 0', "QUORUM = 4"):
        if jalon not in t["analyse"]:
            fautes.append(f"analyse A/B : critere retire ({jalon}).")
    return fautes


def sources() -> str:
    return "\n".join(p.read_text(encoding="utf-8") for p in sorted((RACINE / "src").rglob("*.rs")))


def negatifs(t: dict, src: str) -> list:
    mutations = [
        ("direct par defaut", "gfx", DEFAUT, DEFAUT.replace("false", "true")),
        ("entree sans retour en copie", "gfx", REPART, "fn enter_verrouille() {"),
        ("transfert sans etat", "gpu", "    pose_etat_tampon(en_vol);", ""),
        ("fin sans reponse oubliee", "gpu", "tampons::Fin::SansReponse } else", "tampons::Fin::Achevee } else"),
        ("acces non comptes", "gfx", "            ACCES_EN_TRANSFERT.fetch_add(1, Ordering::Relaxed);\n", ""),
        ("abandon sans quitter", "gfx", "        quitte_memoire_scanout();", ""),
        ("copies non comptees", "gfx", "        OCTETS_COPIES.fetch_add(octets, Ordering::Relaxed);\n    }\n    presente_virtio(0, 0",
         "    }\n    presente_virtio(0, 0"),
        ("regle impure", "pur", "pub fn soumet(", "pub unsafe fn soumet("),
        ("test de transfert retire", "test", "fn aucune_ecriture_pendant_un_transfert()", "fn x()"),
        ("pas KVM retire", "flux", CI[1], "true"),
        ("acces toleres", "analyse", 'd["acces"] != 0', "False"),
        ("commande debranchee", "shell", '"gpu-tampon" => gpu_tampon(argc, &argv),', ""),
    ]
    echecs = []
    for nom, cle, avant, apres in mutations:
        if avant not in t[cle]:
            echecs.append(f"mutation inapplicable : {nom}")
            continue
        m = dict(t)
        m[cle] = t[cle].replace(avant, apres, 1)
        s = src.replace(t[cle], m[cle]) if cle in ("gpu", "gfx", "shell", "pur") else src
        if not verifie(m, s):
            echecs.append(f"mutation non detectee : {nom}")
    s = src + "\nfn x() { crate::drivers::gfx::veut_tampon_direct(true); }\n"
    if not verifie(t, s):
        echecs.append("mutation non detectee : bascule hors du shell")
    return echecs


def main() -> int:
    t = {k: p.read_text(encoding="utf-8") for k, p in FICHIERS.items()}
    src = sources()
    fautes = verifie(t, src) + negatifs(t, src)
    if fautes:
        for f in fautes:
            print(f"verifie-virtio-gpu-tampons: {f}")
        return 1
    print("verifie-virtio-gpu-tampons: OK (copie par defaut, memoire tenue en Transfert autour du transfert, "
          "acces comptes, abandon sans reecriture, copies comptees, regle pure, CI KVM ; 13 negatifs)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
