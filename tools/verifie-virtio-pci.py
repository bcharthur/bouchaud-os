#!/usr/bin/env python3
"""Le transport virtio-PCI ne promet que ce qu'il sert (BOUCHAUD_VIRTIO_PCI_V1).

Premier composant de P10 (docs/ladybird/P10_GPU_AUDIT.md). Regles :

  1. une seule fonctionnalite retenue, VIRTIO_F_VERSION_1 -- accepter VIRGL,
     BLOB ou EVENT_IDX sans les servir serait mentir au peripherique ;
  2. FEATURES_OK est RELU apres ecriture (le peripherique peut refuser) ;
  3. chaque attente est bornee (attente_bornee), aucune boucle nue ;
  4. le marqueur que le banc QEMU exige est present ;
  5. la documentation ne declare pas P10 DONE : ce transport n'accelere rien.
"""
import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
SOURCE = RACINE / "src/drivers/display/virtio_gpu.rs"
README = RACINE / "README.md"
BANC = RACINE / "tools/ci/run_virtio_gpu.sh"


def main() -> int:
    fautes = []
    if not SOURCE.is_file():
        print(f"virtio : {SOURCE} absent")
        return 1
    s = SOURCE.read_text(encoding="utf-8")
    if not re.search(r"let retenues = VERSION_1;", s):
        fautes.append("virtio_gpu.rs : les fonctionnalites retenues ne sont plus VERSION_1 seule.")
    if "l8(DEVICE_STATUS) & FEATURES_OK == 0" not in s:
        fautes.append("virtio_gpu.rs : FEATURES_OK n'est plus relu apres negociation.")
    if s.count("attente_bornee(") < 2:
        fautes.append("virtio_gpu.rs : une attente (reset, reponse) n'est plus bornee.")
    if re.search(r"\bloop\s*\{", s):
        fautes.append("virtio_gpu.rs : boucle `loop` nue (attente non bornee).")
    if "BOUCHAUD_VIRTIO_GPU_OK" not in s:
        fautes.append("virtio_gpu.rs : le marqueur BOUCHAUD_VIRTIO_GPU_OK a disparu.")
    if not BANC.is_file() or "BOUCHAUD_VIRTIO_GPU_OK" not in BANC.read_text(encoding="utf-8"):
        fautes.append("run_virtio_gpu.sh : le banc QEMU n'exige plus BOUCHAUD_VIRTIO_GPU_OK.")
    ligne_p10 = next((l for l in README.read_text(encoding="utf-8").splitlines() if l.startswith("| P10 |")), "")
    if "| DONE |" in ligne_p10:
        fautes.append("README : P10 declare DONE alors que le transport virtio n'accelere rien.")
    if fautes:
        print("virtio-PCI : regle violee")
        for f in fautes:
            print("  " + f)
        return 1
    print("[OK] virtio-PCI : VERSION_1 seule, FEATURES_OK relu, attentes bornees, P10 non pretendu")
    return 0


if __name__ == "__main__":
    sys.exit(main())
