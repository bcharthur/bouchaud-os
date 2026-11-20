#!/usr/bin/env python3
"""Vérifie la reprise des démappages Venus sur deux QEMU compilés à l'identique."""

import argparse
import re
from pathlib import Path
from statistics import median


def charge(dossier):
    return tuple(
        (dossier / nom).read_text(encoding="utf-8", errors="replace")
        for nom in ("qemu.log", "serie.log")
    )


def preuve(trace):
    etats = re.findall(r"virtio_gpu_cmd_res_unmap_blob res (0x[\da-f]+), .*mapping_state (\d+)", trace)
    soumises = re.findall(r"virtio_gpu_fence_ctrl fence (0x[\da-f]+), type 0x209\b", trace)
    acquittees = re.findall(r"virtio_gpu_fence_resp fence (0x[\da-f]+)", trace)
    return etats, soumises, acquittees


def demande(condition, message):
    if not condition:
        raise SystemExit(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("temoin", type=Path, help="journaux du QEMU sans patch")
    parser.add_argument("corrige", type=Path, help="journaux du QEMU avec patch")
    args = parser.parse_args()

    hote, invite = charge(args.temoin)
    etats, soumises, _ = preuve(hote)
    demande(len(etats) == 1 and etats[0][1] == "0" and not soumises,
            "Le témoin doit entrer dans UNMAP puis s'arrêter avant de créer la clôture.")
    demande("VIRTGPU_VENUS_VKFILL_FENCE_OK octets=4096" in invite
            and "VIRTGPU_VENUS_VKFILL_ECHEC etape=17" in invite
            and "VIRTGPU_VENUS_DEMAPPE_OK" not in invite,
            "Le témoin doit comparer le premier motif et échouer lors du démappage.")

    hote, invite = charge(args.corrige)
    etats, soumises, acquittees = preuve(hote)
    demande(len(soumises) >= 2 and len(etats) == 2 * len(soumises),
            "Chaque commande corrigée doit reprendre avant d'armer sa clôture.")
    for (ressource, debut), (reprise, fin) in zip(etats[::2], etats[1::2]):
        demande(ressource == reprise and debut == "0" and fin == "2",
                f"Reprise de démappage incorrecte pour {ressource}.")
    identifiants = set(soumises)
    demande(len(soumises) == len(identifiants)
            and [fence for fence in acquittees if fence in identifiants] == soumises,
            "Il manque un acquittement de clôture ou son ordre est incorrect.")
    demande(invite.count("VIRTGPU_VENUS_VKFILL_FENCE_OK octets=4096") >= 2
            and "VIRTGPU_VENUS_DEMAPPE_OK" in invite
            and "VIRTGPU_VENUS_VKFILL_ECHEC" not in invite,
            "Le banc corrigé doit comparer ses motifs et réussir son nettoyage.")

    mesures = [
        int(us)
        for us in re.findall(r"P10_VENUS_UNMAP reponse=Ok\(4352\) us=(\d+)", invite)
    ]
    demande(bool(mesures), "Mesures de démappage du blob de réponse absentes.")
    print(f"UNMAP clôturés acquittés={len(soumises)} blobs mesurés={len(mesures)} "
          f"minimum_us={min(mesures)} médiane_us={median(mesures):g} maximum_us={max(mesures)}")


if __name__ == "__main__":
    main()
