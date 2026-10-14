#!/usr/bin/env python3
"""Le transport virtio-PCI ne promet que ce qu'il sert (BOUCHAUD_VIRTIO_PCI_V1).

Premier composant de P10 (docs/ladybird/P10_GPU_AUDIT.md). Regles :

  1. une seule fonctionnalite retenue, VIRTIO_F_VERSION_1 -- accepter VIRGL,
     BLOB ou EVENT_IDX sans les servir serait mentir au peripherique ;
  2. FEATURES_OK est RELU apres ecriture (le peripherique peut refuser) ;
  3. chaque attente est bornee : `attente_bornee` (reset) et
     `attente_reponse` (controlq), seule `loop` autorisee, qui porte la borne
     datee ET le plafond TSC ;
  4. le marqueur que le banc QEMU exige est present ;
  5. la documentation ne declare pas P10 DONE : ce transport n'accelere rien ;
  6. BOUCHAUD_VIRTIO_GPU_DMA_ETAT_V1 : une memoire DMA n'est rendue
     (`free_dma`) QUE dans `rend_si_sur`, derriere `dma::peut_rendre` -- un
     `ATTACH_BACKING` sans reponse ne la rend jamais ;
  7. BOUCHAUD_VIRTIO_GPU_VERROU_DORMANT_V1 : le peripherique est sous un
     `SleepMutex`, pris seulement par `verrou()` (`try_lock` hors tache ou
     interruptions masquees) ; plus aucun `SpinLock` tenu pendant une commande ;
     ceder ou dormir seulement si `preempt::cession_volontaire_sure` (aucun
     verrou tournant tenu par l'appelant, IF ouvert, hors IRQ) ;
  8. l'injection de panne (`ATTACH_BACKING` sans reponse) n'existe que sous
     l'option `banc-virtio-gpu-sans-reponse`, hors de `default`, et la CI
     (os-primitives) construit cette image et passe son banc TCG puis KVM.

Fail-closed ; tests negatifs par mutation.
"""
import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
SOURCE = RACINE / "src/drivers/display/virtio_gpu.rs"
README = RACINE / "README.md"
BANC = RACINE / "tools/ci/run_virtio_gpu.sh"
CARGO = RACINE / "Cargo.toml"
FLUX = RACINE / ".github/workflows/os-primitives.yml"
OPTION = "banc-virtio-gpu-sans-reponse"
GARDE_INJECTION = '#[cfg(feature = "' + OPTION + '")]\n        if lit32(requete, 0) == CMD_RESOURCE_ATTACH_BACKING'


def corps(texte: str, nom: str) -> str:
    m = re.search(r"\bfn\s+" + re.escape(nom) + r"\s*(?:<[^>{]*>)?\s*\(", texte)
    if not m:
        return ""
    o = texte.find("{", m.end())
    p = 0
    for i in range(o, len(texte)):
        p += texte[i] == "{"
        p -= texte[i] == "}"
        if p == 0:
            return texte[o:i + 1]
    return ""


def verifie(s: str, readme: str, banc: str, cargo: str = "", flux: str = "") -> list:
    fautes = []
    if not re.search(r"let retenues = VERSION_1;", s):
        fautes.append("virtio_gpu.rs : les fonctionnalites retenues ne sont plus VERSION_1 seule.")
    if "l8(DEVICE_STATUS) & FEATURES_OK == 0" not in s:
        fautes.append("virtio_gpu.rs : FEATURES_OK n'est plus relu apres negociation.")
    if "attente_bornee(1000, || commune.l8(DEVICE_STATUS) == 0)" not in s:
        fautes.append("virtio_gpu.rs : l'attente du reset n'est plus bornee.")
    attente = corps(s, "attente_reponse")
    if not attente or "ecoule > limite_ns" not in attente or "plafond_cycles" not in attente \
            or "return false;" not in attente:
        fautes.append("virtio_gpu.rs : attente_reponse n'est plus bornee (date ET plafond TSC).")
    if "attente_reponse(1000, || read_volatile(util.add(1)) == attendu)" not in s:
        fautes.append("virtio_gpu.rs : la reponse de la controlq n'est plus attendue par attente_reponse borne.")
    hors_attente = s.replace(attente, "") if attente else s
    if re.search(r"\bloop\s*\{", hors_attente):
        fautes.append("virtio_gpu.rs : boucle `loop` nue hors d'attente_reponse (attente non bornee).")
    if "BOUCHAUD_VIRTIO_GPU_OK" not in s:
        fautes.append("virtio_gpu.rs : le marqueur BOUCHAUD_VIRTIO_GPU_OK a disparu.")
    if "BOUCHAUD_VIRTIO_GPU_OK" not in banc:
        fautes.append("run_virtio_gpu.sh : le banc QEMU n'exige plus BOUCHAUD_VIRTIO_GPU_OK.")
    ligne_p10 = next((l for l in readme.splitlines() if l.startswith("| P10 |")), "")
    if "| DONE |" in ligne_p10:
        fautes.append("README : P10 declare DONE alors que le transport virtio n'accelere rien.")
    # 6. DMA
    rend = corps(s, "rend_si_sur")
    if s.count("free_dma(") != 1 or "free_dma(" not in rend \
            or not re.search(r"if dma::peut_rendre\(\*m\) \{\s*memory::free_dma\(", rend):
        fautes.append("virtio_gpu.rs : une memoire DMA est rendue hors de rend_si_sur / sans dma::peut_rendre.")
    if "dma::apres_attache(" not in corps(s, "active_scanout") or "dma::apres_attache(" not in corps(s, "autotest_2d_etapes"):
        fautes.append("virtio_gpu.rs : l'issue d'ATTACH_BACKING n'est plus suivie (scanout ou mire).")
    if "Err(dma::SANS_REPONSE)" not in corps(s, "commande"):
        fautes.append("virtio_gpu.rs : la file ne rend plus SANS_REPONSE sur une attente echue.")
    # 7. Verrou
    if "static GPU: crate::kernel::sync::SleepMutex<Option<Peripherique>>" not in s:
        fautes.append("virtio_gpu.rs : le peripherique n'est plus sous SleepMutex.")
    if re.search(r"SpinLock<Option<Peripherique>>", s):
        fautes.append("virtio_gpu.rs : un SpinLock tient de nouveau le peripherique.")
    v = corps(s, "verrou")
    if s.count("GPU.lock()") != 1 or "GPU.lock()" not in v or "GPU.try_lock()" not in v or "contexte_cedable()" not in v:
        fautes.append("virtio_gpu.rs : le verrou est pris hors de verrou() ou sans repli try_lock.")
    if "crate::kernel::scheduler::preempt::cession_volontaire_sure()" not in corps(s, "contexte_cedable"):
        fautes.append("virtio_gpu.rs : ceder/dormir n'est plus soumis a cession_volontaire_sure (SpinLock tenu ?).")
    # 8. Injection de panne
    if s.count("VIRTIO_GPU_INJECTION") != 1 or s.count(GARDE_INJECTION) != 1:
        fautes.append("virtio_gpu.rs : l'injection de panne n'est plus confinee a l'option " + OPTION + ".")
    defaut = next((l for l in cargo.splitlines() if l.startswith("default")), "")
    if OPTION in defaut or not re.search(r"^" + re.escape(OPTION) + r" = \[\]$", cargo, re.M):
        fautes.append("Cargo.toml : l'option " + OPTION + " manque ou entre dans default.")
    if "cargo bootimage --features " + OPTION not in flux or flux.count("run_virtio_gpu_dma_panne.sh") != 2 \
            or "BO_QEMU_KVM=1 tools/ci/run_virtio_gpu_dma_panne.sh" not in flux:
        fautes.append("os-primitives.yml : le banc de panne DMA n'est plus passe (TCG puis KVM).")
    return fautes


def main() -> int:
    if not SOURCE.is_file():
        print(f"virtio : {SOURCE} absent")
        return 1
    s = SOURCE.read_text(encoding="utf-8")
    readme = README.read_text(encoding="utf-8")
    banc = BANC.read_text(encoding="utf-8") if BANC.is_file() else ""
    cargo = CARGO.read_text(encoding="utf-8")
    flux = FLUX.read_text(encoding="utf-8")
    fautes = verifie(s, readme, banc, cargo, flux)
    if fautes:
        print("virtio-PCI : regle violee")
        for f in fautes:
            print("  " + f)
        return 1
    negatifs = [
        # rendre la memoire sur le seul « jamais attachee » (le defaut)
        ("if dma::peut_rendre(*m) {", "if *m != Memoire::Attachee {"),
        # retirer la borne datee de l'attente de reponse
        ("if ecoule > limite_ns || ", "if "),
        # revenir au SpinLock
        ("static GPU: crate::kernel::sync::SleepMutex<Option<Peripherique>> = crate::kernel::sync::SleepMutex::new(None);",
         "static GPU: crate::kernel::sync::SpinLock<Option<Peripherique>> = crate::kernel::sync::SpinLock::new(None);"),
        # ne plus suivre l'issue de l'attache du bureau
        ("                memoire = dma::apres_attache(memoire, dma::issue(rendu));\n", ""),
        # ceder sans verifier qu'aucun verrou tournant n'est tenu
        ("    crate::kernel::scheduler::preempt::cession_volontaire_sure()\n",
         "    crate::arch::x86_64::cpu::interrupts_enabled() && crate::kernel::task::in_user_task()\n"),
        # l'injection compilee dans toute image
        ('#[cfg(feature = "' + OPTION + '")]\n        if lit32', "if lit32"),
    ]
    for n, (a, b) in enumerate(negatifs, 1):
        if a not in s or not verifie(s.replace(a, b, 1), readme, banc, cargo, flux):
            print(f"virtio-PCI : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    autres = [
        ("Cargo.toml", lambda: verifie(s, readme, banc, cargo.replace('default = ["legacy-boot"]', 'default = ["legacy-boot", "' + OPTION + '"]', 1), flux)),
        ("os-primitives.yml", lambda: verifie(s, readme, banc, cargo, flux.replace("BO_QEMU_KVM=1 tools/ci/run_virtio_gpu_dma_panne.sh", "true", 1))),
    ]
    for nom, f in autres:
        if not f():
            print(f"virtio-PCI : la mutation de {nom} ne rougit pas -- garde inoperant")
            return 1
    print(f"[OK] virtio-PCI : VERSION_1 seule, FEATURES_OK relu, attentes bornees, DMA rendue seulement si sure, "
          f"verrou dormant, injection confinee et passee en CI, P10 non pretendu (negatifs={len(negatifs) + len(autres)})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
