#!/usr/bin/env python3
"""L'achevement des commandes virtio-gpu par interruption reste sur
(BOUCHAUD_VIRTIO_GPU_IRQ_V1, P10+.1).

Regles :

  1. le gestionnaire (`virtio_gpu::interruption`) ne prend aucun verrou,
     n'imprime rien, n'envoie aucune commande : il compte et reveille la
     file des achevements ; `idt/virtio_gpu.rs` l'appelle puis acquitte le
     LAPIC, et `idt.rs` l'installe sur `virtio_gpu_irq::VECTEUR` ;
  2. pas de reveil perdu : dans `MondeNoyau::dors`, le ticket est pris
     AVANT la derniere relecture, et le sommeil porte CE ticket ;
  3. on ne dort que si `irq::strategie` le permet (contexte cedable,
     vecteur accepte, politique) ; sinon la scrutation bornee d'avant ;
  4. l'armement relit `queue_msix_vector` et desactive MSI-X sur un refus ;
  5. la politique par defaut est l'INTERRUPTION (`IRQ_VOULUE` vrai) depuis
     l'A/B KVM CONVAINCANT de f23a1247 (run 38697040371) -- le commentaire
     de la statique doit citer ce run ; la rendre a la scrutation exige de
     changer ce garde-fou, avec la preuve ;
  6. la CI mesure l'A/B sous KVM et rejoue les pannes (option de banc hors
     `default`).

Fail-closed ; tests negatifs par mutation.
"""
import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
FICHIERS = {
    "pilote": RACINE / "src/drivers/display/virtio_gpu.rs",
    "pur": RACINE / "src/drivers/display/virtio_gpu_irq.rs",
    "idt": RACINE / "src/arch/x86_64/idt.rs",
    "gest": RACINE / "src/arch/x86_64/idt/virtio_gpu.rs",
    "flux": RACINE / ".github/workflows/os-primitives.yml",
    "cargo": RACINE / "Cargo.toml",
}


def corps(texte: str, entete: str) -> str:
    """Le corps d'une fonction : jusqu'a l'accolade de MEME indentation."""
    i = texte.find(entete)
    if i < 0:
        return ""
    retrait = entete[:len(entete) - len(entete.lstrip())]
    j = texte.find("\n" + retrait + "}\n", i)
    return texte[i:j] if j > 0 else ""


def verifie(t: dict) -> list:
    fautes = []
    g = corps(t["pilote"], "pub fn interruption() {")
    if not g or "ACHEVEMENTS.wake_all();" not in g:
        fautes.append("interruption() : ne reveille plus la file des achevements.")
    if re.search(r"\.lock\(|try_lock|println!|commande\(|yield_now|wait", g):
        fautes.append("interruption() : verrou, impression, commande ou attente dans le gestionnaire.")
    if "crate::drivers::virtio_gpu::interruption();" not in t["gest"] or "smp::eoi_local();" not in t["gest"]:
        fautes.append("idt/virtio_gpu.rs : le gestionnaire n'appelle plus interruption() ou n'acquitte plus le LAPIC.")
    if "IDT[crate::drivers::virtio_gpu_irq::VECTEUR as usize].set_handler_fn(virtio_gpu_interrupt_handler);" not in t["idt"] \
            or 'include!("idt/virtio_gpu.rs");' not in t["idt"]:
        fautes.append("idt.rs : le vecteur virtio-gpu n'est plus installe.")
    d = corps(t["pilote"], "    fn dors(&mut self, jusqu_a: u64, pret: &mut dyn FnMut() -> bool) -> Reveil {")
    a, b, c = d.find("let ticket = ACHEVEMENTS.ticket();"), d.find("if pret() {"), d.find("ACHEVEMENTS.wait_until_noyau(ticket, jusqu_a)")
    if min(a, b, c) < 0 or not (a < b < c):
        fautes.append("dors() : le ticket n'est plus pris avant la derniere relecture (reveil perdu possible).")
    w = corps(t["pilote"], "fn attente_achevement(")
    if "contexte_cedable()," not in w or "if strategie == Strategie::Scrute {\n        return attente_reponse(limite_ms, predicat);" not in w:
        fautes.append("attente_achevement() : sommeil hors de irq::strategie, ou plus de repli sur la scrutation.")
    if "if cedable && armee && voulue {\n        Strategie::Dort" not in t["pur"]:
        fautes.append("virtio_gpu_irq.rs : strategie() dort sans exiger contexte cedable, vecteur accepte et politique.")
    m = corps(t["pilote"], "unsafe fn arme_msix(")
    if "if commune.l16(QUEUE_MSIX_VECTOR) != 0 {" not in m or "ecrit_controle(controle & !irq::MSIX_ACTIVE);" not in m:
        fautes.append("arme_msix() : un refus du peripherique n'est plus detecte ou MSI-X reste actif.")
    if "static IRQ_VOULUE: AtomicBool = AtomicBool::new(true);" not in t["pilote"] \
            or "CONVAINCANT de f23a1247 (run 38697040371" not in t["pilote"]:
        fautes.append("politique par defaut : l'interruption (A/B KVM CONVAINCANT, run 38697040371) n'est plus le defaut, "
                      "ou la preuve n'est plus citee.")
    if "BO_QEMU_KVM=1 tools/ci/run_banc_irq_gpu.sh" not in t["flux"] \
            or "cargo bootimage --features banc-virtio-gpu-irq-pannes" not in t["flux"] \
            or "BO_QEMU_KVM=1 tools/ci/run_virtio_gpu_irq_pannes.sh" not in t["flux"]:
        fautes.append("os-primitives.yml : l'A/B KVM ou le banc de pannes n'est plus execute.")
    # BOUCHAUD_P10_HORS_VOL_SILENCE_V1 : une parasite n'est hors vol qu'apres un
    # silence du gestionnaire, mesure AVANT de l'envoyer (exp/p10-pannes-hors-vol).
    h = corps(t["pilote"], "    pub(super) fn apres_attente(rang: u64, reussi: bool) {")
    s_, e_ = h.find("let mut calme = crate::kernel::timer::monotonic_ns().saturating_add(SILENCE_NS);"), h.find("HORS_VOL.fetch_add(1, Ordering::Relaxed);")
    if min(s_, e_) < 0 or not s_ < e_ or "const SILENCE_NS: u64 = 200_000;" not in t["pilote"] \
            or "if total != vu {" not in h \
            or "calme = crate::kernel::timer::monotonic_ns().saturating_add(SILENCE_NS);\n            }" not in h:
        fautes.append("banc de pannes : la parasite hors vol n'attend plus un silence du gestionnaire "
                      "(BOUCHAUD_P10_HORS_VOL_SILENCE_V1) -- une MSI-X tardive retombe dans la fenetre.")
    defaut = re.search(r"^default = \[([^\]]*)\]", t["cargo"], re.M)
    if not defaut or "banc-virtio-gpu-irq-pannes" in defaut.group(1) or "banc-virtio-gpu-irq-pannes = []" not in t["cargo"]:
        fautes.append("Cargo.toml : l'option de banc est absente ou dans `default`.")
    return fautes


def main() -> int:
    t = {}
    for nom, chemin in FICHIERS.items():
        if not chemin.is_file():
            print(f"virtio-gpu irq : {chemin} absent")
            return 1
        t[nom] = chemin.read_text(encoding="utf-8")
    fautes = verifie(t)
    if fautes:
        print("virtio-gpu irq : regle violee")
        for f in fautes:
            print("  " + f)
        return 1
    negatifs = [
        ("pilote", "    ACHEVEMENTS.wake_all();\n}", "    let _ = GPU.lock();\n    ACHEVEMENTS.wake_all();\n}"),
        ("pilote", "    ACHEVEMENTS.wake_all();\n}", "    crate::serial_println!(\"irq\");\n    ACHEVEMENTS.wake_all();\n}"),
        ("gest", "    smp::eoi_local();", ""),
        ("pilote", "        let ticket = ACHEVEMENTS.ticket();\n        if pret() {",
         "        if pret() {\n            return Reveil::DejaPret;\n        }\n        let ticket = ACHEVEMENTS.ticket();\n        if pret() {"),
        ("pilote", "    if strategie == Strategie::Scrute {\n        return attente_reponse(limite_ms, predicat);\n    }", ""),
        ("pur", "if cedable && armee && voulue {", "if armee && voulue {"),
        ("pilote", "        ecrit_controle(controle & !irq::MSIX_ACTIVE);\n", ""),
        ("pilote", "static IRQ_VOULUE: AtomicBool = AtomicBool::new(true);", "static IRQ_VOULUE: AtomicBool = AtomicBool::new(false);"),
        ("flux", "BO_QEMU_KVM=1 tools/ci/run_banc_irq_gpu.sh", "tools/ci/run_banc_irq_gpu.sh"),
        ("cargo", 'default = ["legacy-boot"]', 'default = ["legacy-boot", "banc-virtio-gpu-irq-pannes"]'),
        ("pilote", "            if total != vu {", "            if false {"),
        ("pilote", "    const SILENCE_NS: u64 = 200_000;", "    const SILENCE_NS: u64 = 0;"),
    ]
    for n, (cle, a, b) in enumerate(negatifs, 1):
        if a not in t[cle]:
            print(f"virtio-gpu irq : le test negatif {n} ne s'applique plus ({cle}) -- garde inoperant")
            return 1
        mute = dict(t)
        mute[cle] = t[cle].replace(a, b, 1)
        if not verifie(mute):
            print(f"virtio-gpu irq : le test negatif {n} ne rougit pas ({cle}) -- garde inoperant")
            return 1
    print(f"[OK] virtio-gpu irq : gestionnaire sans verrou ni impression, ticket avant relecture, sommeil seulement "
          f"si cedable/arme/voulu, refus MSI-X detecte, interruption par defaut (A/B KVM 38697040371), A/B KVM et pannes en CI, "
          f"parasite hors vol apres silence "
          f"(negatifs={len(negatifs)})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
