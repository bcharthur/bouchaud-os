#!/usr/bin/env python3
"""BOUCHAUD_LIVRAISON_V1 -- l'image de LIVRAISON, demarree telle quelle, utilisee comme un humain.

    test_livraison_qemu.py OUT_DIR [--exige-ladybird]

OUT_DIR : une sortie de tools/release/build_trigkey_ab_image.sh. Le test ne
reconstruit RIEN : il demarre une copie jetable de `disque-ab.img` (empreinte
verifiee avant et apres) sous QEMU/OVMF, disque NVMe, carte e1000, puis, avec
la souris de QEMU (tools/ci/qemu_gestes.py) et la geometrie que l'Atelier
journalise lui-meme (BOUCHAUD_ATELIER_DEMARRER, BOUCHAUD_ATELIER_MENU_LIGNE) :
  - ouvre le menu Demarrer et lance le Terminal : sa fenetre doit s'ouvrir ;
  - lance DOOM : processus ring 3 lance ET premiere trame livree a l'Atelier ;
  - lance Ladybird (si embarque) : processus lance, premiere trame livree, et
    premiere trame de son interface (BOUCHAUD_UI_FIRST_FRAME) ;
et exige : slot A demarre, services en bonne sante, STATE montee, agent OTA
actif sur le canal stable, aucune panique. Captures d'ecran conservees.

Verdict :
  LIVRAISON_QEMU_OK      tout ce qui est embarque fonctionne, image LIVRABLE ;
  LIVRAISON_QEMU_PARTIEL tout ce qui est embarque fonctionne, mais l'image
                         n'est pas livrable (Ladybird absent) -- code 0 sauf
                         --exige-ladybird (code 1) ;
  LIVRAISON_QEMU_ECHEC   un controle a echoue (code 1).
PHYSICAL=NOT_RUN : QEMU ne prouve rien du Trigkey.
"""
from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

RACINE = Path(__file__).resolve().parents[2]
GESTES = RACINE / "tools/ci/qemu_gestes.py"
ANSI = re.compile(r"\x1b\[[0-9;]*m")
OVMF = Path(os.environ.get("BO_OVMF_DIR", "/usr/share/OVMF"))
DELAI_BUREAU = int(os.environ.get("BO_LIVRAISON_BUREAU_S", "420"))
DELAI_LADYBIRD = int(os.environ.get("BO_LIVRAISON_LADYBIRD_S", "900"))


def sha(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for b in iter(lambda: f.read(1 << 22), b""):
            h.update(b)
    return h.hexdigest()


# Bouchaud OS decode le clavier en AZERTY-FR (src/drivers/input/clavier_decodeur.rs) ;
# `sendkey` nomme des touches PHYSIQUES (positions QWERTY US). Taper `bo-update`
# avec les noms QWERTY donnait `bo)updqte` (premier essai, capture 02b). Table
# explicite : caractere -> touche physique qui le produit en AZERTY.
TOUCHES_AZERTY = {
    "a": "q", "q": "a", "z": "w", "w": "z", "m": "semicolon",
    "-": "6", "_": "8", "=": "equal", " ": "spc",
    ".": "shift-comma", "/": "shift-dot", ":": "dot", ",": "m", ";": "comma",
    **{str(n): f"shift-{n}" for n in range(10)},
}


def tape(texte: str) -> list[str]:
    """Le texte en commandes `sendkey` du moniteur QEMU (AZERTY), puis Entree."""
    touches = []
    for c in texte:
        if c in TOUCHES_AZERTY:
            touches.append(TOUCHES_AZERTY[c])
        elif "a" <= c <= "z":
            touches.append(c)
        else:
            raise ValueError(f"caractere sans touche AZERTY connue : {c!r}")
    return [f"sendkey {t}" for t in touches] + ["sendkey ret"]


def contexte(texte: str, motif: str, n: int = 3, autour: int = 4) -> None:
    """Les n premieres occurrences de `motif`, avec `autour` lignes de part et
    d'autre, dans le journal du job : un controle rouge doit dire POURQUOI
    sans exiger l'artefact (run 43263711449 : « LOCKDEP » sans panique)."""
    lignes = texte.splitlines()
    vus = 0
    for i, l in enumerate(lignes):
        if motif in l:
            for j in range(max(0, i - autour), min(len(lignes), i + autour + 1)):
                print(f"LIVRAISON_CONTEXTE {motif!r} {j + 1}: {lignes[j][:300]}", flush=True)
            vus += 1
            if vus >= n:
                break


class Banc:
    def __init__(self, out: Path, travail: Path):
        self.out, self.travail = out, travail
        self.log = out / "livraison-boot.log"
        self.sock = travail / "moniteur.sock"
        self.captures = out / "livraison-captures"
        self.captures.mkdir(exist_ok=True)
        self.erreurs: list[str] = []
        self.ok: list[str] = []
        self.qemu: subprocess.Popen | None = None

    def texte(self) -> str:
        try:
            return ANSI.sub("", self.log.read_text(errors="replace")).replace("\r", "")
        except OSError:
            return ""

    def attend(self, motif: str, delai: float, apres: int = 0) -> re.Match | None:
        fin = time.time() + delai
        rx = re.compile(motif)
        while time.time() < fin:
            m = rx.search(self.texte(), apres)
            if m:
                return m
            if self.qemu and self.qemu.poll() is not None:
                return None
            time.sleep(1)
        return None

    def gestes(self, *lignes: str) -> None:
        subprocess.run([sys.executable, str(GESTES), str(self.sock), str(self.captures)],
                       input="\n".join(lignes) + "\n", text=True, check=True, timeout=120,
                       capture_output=True)

    def demarre(self) -> None:
        disque = self.travail / "essai.img"
        shutil.copyfile(self.out / "disque-ab.img", disque)
        shutil.copyfile(OVMF / "OVMF_VARS_4M.fd", self.travail / "vars.fd")
        accel = ["-cpu", "max"]
        if os.environ.get("BO_QEMU_KVM") == "1" and os.access("/dev/kvm", os.W_OK):
            accel = ["-enable-kvm", "-cpu", "host"]
        self.log.write_text("")
        self.qemu = subprocess.Popen(
            ["qemu-system-x86_64", "-machine", "q35", "-m", "4096", "-smp", "2", *accel,
             "-drive", f"if=pflash,format=raw,unit=0,readonly=on,file={OVMF / 'OVMF_CODE_4M.fd'}",
             "-drive", f"if=pflash,format=raw,unit=1,file={self.travail / 'vars.fd'}",
             "-drive", f"if=none,id=disque,format=raw,file={disque}",
             "-device", "nvme,drive=disque,serial=BOUCHAUD_LIVRAISON",
             "-nic", "user,model=e1000", "-vga", "std", "-display", "none", "-no-reboot",
             "-monitor", f"unix:{self.sock},server=on,wait=off",
             "-serial", f"file:{self.log}"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def arrete(self) -> None:
        if self.qemu and self.qemu.poll() is None:
            self.qemu.kill()
            self.qemu.wait()

    def note(self, ok: bool, quoi: str) -> bool:
        (self.ok if ok else self.erreurs).append(quoi)
        print(("LIVRAISON_OK   " if ok else "LIVRAISON_FAIL ") + quoi, flush=True)
        return ok

    def ouvre_par_le_menu(self, libelle: str) -> bool:
        """Clique sur Demarrer (geometrie journalisee pour le nombre de fenetres
        COURANT), puis sur la ligne `libelle` du menu."""
        t = self.texte()
        dem = re.findall(r"BOUCHAUD_ATELIER_DEMARRER fenetres=(\d+) x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)", t)
        if not dem:
            return self.note(False, f"{libelle} : geometrie du bouton Demarrer absente")
        _, x, y, w, h = map(int, dem[-1])
        avant = len(t)
        self.gestes(f"va {x + w // 2} {y + h // 2}", "attente 0.3", "clic", "attente 1")
        m = self.attend(rf"BOUCHAUD_ATELIER_MENU_LIGNE libelle={re.escape(libelle)} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)", 30, avant)
        if not m:
            return self.note(False, f"{libelle} : le menu Demarrer ne s'est pas ouvert")
        lx, ly, lw, lh = map(int, m.groups())
        self.gestes(f"va {lx + lw // 2} {ly + lh // 2}", "attente 0.3", "clic")
        return True


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    out = Path(argv[1]).resolve()
    exige_ladybird = "--exige-ladybird" in argv[2:]
    for f in ("disque-ab.img", "disque-ab.img.sha256", "composants.txt"):
        if not (out / f).is_file():
            print(f"LIVRAISON_REFUSE {f}_absent", file=sys.stderr)
            return 2
    attendu = (out / "disque-ab.img.sha256").read_text().split()[0]
    if sha(out / "disque-ab.img") != attendu:
        print("LIVRAISON_REFUSE empreinte_initiale_differente", file=sys.stderr)
        return 2
    composants = dict(l.split("=", 1) for l in (out / "composants.txt").read_text().splitlines() if "=" in l)
    ladybird = composants.get("ladybird") == "present"

    with tempfile.TemporaryDirectory() as t:
        b = Banc(out, Path(t))
        b.demarre()
        try:
            b.note(b.attend(r"BOUCHAUD_AB_BOOT slot=A", 300) is not None, "chargeur A/B : slot A demarre")
            pret = b.attend(r"BOUCHAUD_ATELIER_DEMARRER fenetres=0 ", DELAI_BUREAU)
            if b.note(pret is not None, "bureau Atelier affiche (premiere trame, Demarrer place)"):
                b.gestes("attente 2", "capture 01-bureau")
                b.note(b.attend(r"SERVICED_HEALTH_COMMIT .*result=already_good", 120) is not None,
                       "services critiques en bonne sante (serviced)")
                b.note(b.attend(r"BOUCHAUD_PERSIST_MONTAGE etat=(neuf|restaure)", 120) is not None,
                       "STATE montee (persistance)")
                b.note(b.attend(r"BOUCHAUD_OTA_(VERIFIE|CYCLE|INACTIF raison=sans_reseau)|BOUCHAUD_OTA_CYCLE", 240) is not None
                       and "BOUCHAUD_OTA_INACTIF raison=sans_source" not in b.texte(),
                       "agent OTA actif (canal stable configure)")
                # 1. Terminal
                if b.ouvre_par_le_menu("Terminal"):
                    if b.note(b.attend(r"BOUCHAUD_ATELIER_FENETRE_OUVERTE titre=Terminal", 30) is not None,
                              "Terminal ouvert par le menu Demarrer"):
                        b.gestes("attente 1", "capture 02-terminal")
                        # La commande de l'etat des mises a jour, TAPEE au clavier.
                        b.gestes(*tape("bo-update status"), "attente 2", "capture 02b-bo-update")
                        b.note(b.attend(r"BOUCHAUD_BO_UPDATE_STATUS version=\S+ commit=[0-9a-f]{40} slot_actif=A canal=stable", 60) is not None,
                               "bo-update status tape dans le Terminal : version, commit, slot A, canal stable")
                # 2. DOOM
                if b.ouvre_par_le_menu("DOOM"):
                    m = b.attend(r"BOUCHAUD_DOOM_ATELIER_LAUNCH pid=(\d+)", 60)
                    if b.note(m is not None, "DOOM lance (processus ring 3)"):
                        pid = m.group(1)
                        if b.note(b.attend(rf"BOUCHAUD_ATELIER_CLIENT_PREMIERE_TRAME pid={pid} ", 180) is not None,
                                  f"DOOM affiche (premiere trame, pid {pid})"):
                            b.gestes("attente 3", "capture 03-doom")
                # 3. Ladybird
                if ladybird:
                    if b.ouvre_par_le_menu("Ladybird"):
                        m = b.attend(r"BOUCHAUD_STAGE2_LADYBIRD_LAUNCHED pid=(\d+)", 120)
                        if b.note(m is not None, "Ladybird lance (processus ring 3)"):
                            pid = m.group(1)
                            b.note(b.attend(rf"BOUCHAUD_ATELIER_CLIENT_PREMIERE_TRAME pid={pid} ", DELAI_LADYBIRD) is not None,
                                   f"Ladybird affiche (premiere trame, pid {pid})")
                            if b.note(b.attend(r"BOUCHAUD_UI_FIRST_FRAME onglet=1", DELAI_LADYBIRD) is not None,
                                      "Ladybird : interface et premier onglet rendus"):
                                b.gestes("attente 5", "capture 04-ladybird")
                else:
                    print("LIVRAISON_ABSENT Ladybird n'est pas embarque dans cette image (composants.txt)", flush=True)
                time.sleep(5)
            texte = b.texte()
            for x in ("*** KERNEL PANIC ***", "DOUBLE FAULT", "TRIPLE FAULT", "LOCKDEP"):
                if not b.note(x not in texte, f"aucune ligne « {x} »"):
                    contexte(texte, x)
        finally:
            b.arrete()
        b.note(sha(out / "disque-ab.img") == attendu, "image de livraison inchangee (essai sur copie)")

    livrable = composants.get("livrable") == "oui" and ladybird
    if b.erreurs:
        verdict = "LIVRAISON_QEMU_ECHEC"
    elif livrable:
        verdict = "LIVRAISON_QEMU_OK"
    else:
        verdict = "LIVRAISON_QEMU_PARTIEL"
    preuve = {"schema": "bouchaud-livraison-qemu/1", "verdict": verdict, "composants": composants,
              "controles_ok": b.ok, "controles_echec": b.erreurs,
              "captures": sorted(p.name for p in (out / "livraison-captures").glob("*.ppm")),
              "image_sha256": attendu, "physical": "NOT_RUN"}
    (out / "livraison-evidence.json").write_text(json.dumps(preuve, indent=2, ensure_ascii=False) + "\n")
    print(f"{verdict} controles={len(b.ok)} echecs={len(b.erreurs)} ladybird={'present' if ladybird else 'absent'} PHYSICAL=NOT_RUN")
    if b.erreurs or (exige_ladybird and not livrable):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
