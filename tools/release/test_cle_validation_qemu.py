#!/usr/bin/env python3
"""BOUCHAUD_CLE_VALIDATION_V1 -- la cle de VALIDATION du Trigkey, demarree telle quelle.

    test_cle_validation_qemu.py USB_DIR

USB_DIR : sortie de `BO_CLE_USB_TEST=1 tools/release/build_trigkey_image.sh`,
la commande que lance `tools/windows/Prepare-TrigkeyUSB.ps1 -Mode Validation`
(systeme assemble par tools/release/assemble_systeme.sh). Rien n'est
reconstruit : une COPIE de `bouchaud-trigkey.img` (empreinte verifiee avant et
apres) demarre sous QEMU/OVMF comme une CLE USB (xHCI, usb-storage, amorcage),
a cote d'un disque NVMe INTERNE rempli d'octets connus -- le disque d'origine
du Trigkey. Puis, comme un humain (souris et clavier de QEMU) :
  - bureau Atelier affiche ;
  - regle `cle-usb-test` active (lecture_seule=1, media autorise = la cle) ;
  - reseau : bail DHCP ;
  - Terminal ouvert par le menu Demarrer ;
  - DOOM lance et affiche (premiere trame) ;
et exige : le disque NVMe interne a la MEME empreinte apres la session
qu'avant, aucune panique.

Ce banc ne remplace pas tools/ci/run_cle_usb_test.sh (sonde d'ecriture
active, trace des commandes recues par QEMU, temoin negatif) : il prouve que
la cle CONSTRUITE pour l'utilisateur demarre et sert, sans toucher au disque
interne. PHYSICAL=NOT_RUN.
"""
from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import test_livraison_qemu as tl  # noqa: E402

DELAI_BUREAU = int(os.environ.get("BO_LIVRAISON_BUREAU_S", "420"))


class BancCle(tl.Banc):
    def __init__(self, usb: Path, travail: Path):
        super().__init__(usb, travail)
        self.log = usb / "validation-boot.log"
        self.captures = usb / "validation-captures"
        self.captures.mkdir(exist_ok=True)
        self.interne = travail / "nvme-interne.img"

    def demarre(self) -> None:
        cle = self.travail / "cle.img"
        shutil.copyfile(self.out / "bouchaud-trigkey.img", cle)
        shutil.copyfile(tl.OVMF / "OVMF_VARS_4M.fd", self.travail / "vars.fd")
        # Le disque d'origine du Trigkey : une table GPT valide et des octets connus.
        subprocess.run([sys.executable, str(tl.RACINE / "tools/ci/fabrique-disque-gpt.py"), str(self.interne), "--mio", "64"],
                       check=True, capture_output=True)
        with open(self.interne, "r+b") as f:
            f.seek(2 * 1024 * 1024)
            f.write(b"BOUCHAUD_DONNEES_INTERNES_A_PRESERVER" * 4096)
        accel = ["-cpu", "max"]
        if os.environ.get("BO_QEMU_KVM") == "1" and os.access("/dev/kvm", os.W_OK):
            accel = ["-enable-kvm", "-cpu", "host"]
        self.log.write_text("")
        self.qemu = subprocess.Popen(
            ["qemu-system-x86_64", "-machine", "q35", "-m", "4096", "-smp", "2", *accel,
             "-drive", f"if=pflash,format=raw,unit=0,readonly=on,file={tl.OVMF / 'OVMF_CODE_4M.fd'}",
             "-drive", f"if=pflash,format=raw,unit=1,file={self.travail / 'vars.fd'}",
             "-device", "qemu-xhci,id=xhci",
             "-drive", f"if=none,id=cle,format=raw,file={cle}",
             "-device", "usb-storage,bus=xhci.0,drive=cle,removable=on,bootindex=0",
             "-drive", f"if=none,id=interne,format=raw,file={self.interne}",
             "-device", "nvme,drive=interne,serial=TRIGKEY_INTERNE",
             "-nic", "user,model=e1000", "-vga", "std", "-display", "none", "-no-reboot",
             "-monitor", f"unix:{self.sock},server=on,wait=off",
             "-serial", f"file:{self.log}"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def note(self, ok: bool, quoi: str) -> bool:
        (self.ok if ok else self.erreurs).append(quoi)
        print(("VALIDATION_OK   " if ok else "VALIDATION_FAIL ") + quoi, flush=True)
        return ok


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    usb = Path(argv[1]).resolve()
    image = usb / "bouchaud-trigkey.img"
    for f in (image, usb / "bouchaud-trigkey.img.sha256", usb / "manifest.json"):
        if not f.is_file():
            print(f"VALIDATION_REFUSE {f.name}_absent", file=sys.stderr)
            return 2
    attendu = (usb / "bouchaud-trigkey.img.sha256").read_text().split()[0]
    if tl.sha(image) != attendu:
        print("VALIDATION_REFUSE empreinte_initiale_differente", file=sys.stderr)
        return 2
    manifeste = json.loads((usb / "manifest.json").read_text())
    if "cle-usb-test" not in manifeste.get("build", {}).get("features", []):
        print("VALIDATION_REFUSE image_sans_cle_usb_test (BO_CLE_USB_TEST=1 exige)", file=sys.stderr)
        return 2
    guid = manifeste["cle_usb_test"]["media_guid_gpt"]

    with tempfile.TemporaryDirectory() as t:
        b = BancCle(usb, Path(t))
        b.demarre()
        avant = None
        try:
            # L'empreinte du disque interne est prise APRES sa fabrication.
            avant = tl.sha(b.interne)
            b.note(b.attend(r"BOUCHAUD_DISQUES_INTERNES lecture_seule=1 image=cle-usb-test media_autorise=guid-gpt", 300) is not None,
                   "regle cle-usb-test active (disques internes en lecture seule, media autorise par GUID GPT)")
            pret = b.attend(r"BOUCHAUD_ATELIER_DEMARRER fenetres=0 ", DELAI_BUREAU)
            if b.note(pret is not None, "bureau Atelier affiche depuis la cle USB"):
                b.gestes("attente 2", "capture 01-bureau")
                b.note(b.attend(r"phase=IPV4_READY", 120) is not None, "reseau : bail DHCP obtenu")
                b.note(b.attend(r"BOUCHAUD_MEDIA_IDENTITE support=usb .*identite=autorisee ", 60) is not None,
                       f"la cle de demarrage est reconnue comme le media autorise (GUID {guid[:8]}...)")
                if b.ouvre_par_le_menu("Terminal"):
                    if b.note(b.attend(r"BOUCHAUD_ATELIER_FENETRE_OUVERTE titre=Terminal", 30) is not None,
                              "Terminal ouvert par le menu Demarrer"):
                        b.gestes("attente 1", "capture 02-terminal")
                if b.ouvre_par_le_menu("DOOM"):
                    m = b.attend(r"BOUCHAUD_DOOM_ATELIER_LAUNCH pid=(\d+)", 60)
                    if b.note(m is not None, "DOOM lance (processus ring 3)"):
                        if b.note(b.attend(rf"BOUCHAUD_ATELIER_CLIENT_PREMIERE_TRAME pid={m.group(1)} ", 180) is not None,
                                  "DOOM affiche (premiere trame)"):
                            b.gestes("attente 3", "capture 03-doom")
                time.sleep(5)
            texte = b.texte()
            for x in ("*** KERNEL PANIC ***", "DOUBLE FAULT", "TRIPLE FAULT") + tl.LOCKDEP_FATAL:
                if not b.note(x not in texte, f"aucune ligne « {x} »"):
                    tl.contexte(texte, x)
            tl.controle_lockdep(b, texte)
            refus = re.findall(r"BOUCHAUD_ECRITURE_INTERNE_REFUSEE support=(\w+)", texte)
            print(f"VALIDATION_INFO refus_d_ecriture_interne={len(refus)} supports={sorted(set(refus))}", flush=True)
        finally:
            b.arrete()
        b.note(avant is not None and tl.sha(b.interne) == avant, "disque NVMe INTERNE inchange (meme empreinte avant/apres)")
        b.note(tl.sha(image) == attendu, "image de la cle inchangee (essai sur copie)")

    verdict = "CLE_VALIDATION_QEMU_ECHEC" if b.erreurs else "CLE_VALIDATION_QEMU_OK"
    preuve = {"schema": "bouchaud-cle-validation-qemu/1", "verdict": verdict, "commit": manifeste.get("bouchaud_commit"),
              "image_sha256": attendu, "media_guid_gpt": guid, "controles_ok": b.ok, "controles_echec": b.erreurs,
              "captures": sorted(p.name for p in (usb / "validation-captures").glob("*.ppm")), "physical": "NOT_RUN"}
    (usb / "validation-evidence.json").write_text(json.dumps(preuve, indent=2, ensure_ascii=False) + "\n")
    print(f"{verdict} controles={len(b.ok)} echecs={len(b.erreurs)} PHYSICAL=NOT_RUN")
    return 1 if b.erreurs else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
