#!/usr/bin/env python3
"""BOUCHAUD_CLE_USB_TEST_V1 : aucune commande de stockage ne contourne la regle.

La regle (`src/drivers/api/stockage_regle.rs`) ne vaut que si TOUTE commande
passe par elle. Ce garde verifie, sur les sources :

  1. ATA : `outb(COMMAND` n'apparait qu'UNE fois dans tout `src/`, dans
     `emet_commande`, apres `ecriture_interne::ata(` ; aucun port de commande
     ATA en dur (`0x1F7`, `0x177`) n'est ECRIT ailleurs ;
  2. NVMe : l'entree de soumission n'est ecrite (`write_volatile(emplacement`)
     que dans `File::pose`, apres `ecriture_interne::nvme(` ; `admin` y est
     deduit de l'identifiant de la file (`admin: file == 0`), jamais passe a
     part ; `pose` n'est appelee qu'a travers `emet` et `soumet_es` ;
  3. NVMe : un echec de soumission rend l'identifiant (`jamais_pose`) ;
  4. USB : `encode_cbw(` n'apparait qu'aux deux transports (volume et boite
     noire), chacun APRES `ecriture_interne::scsi(` dans la meme fonction ;
     l'identite est etablie AVANT la sonde de vidange du montage ;
  5. `cle-usb-test` et `banc-ecriture-interne` ne sont pas dans `default` ;
     la sonde n'est compilee que derriere `banc-ecriture-interne` et refuse
     de tourner hors d'une VM QEMU ;
  6. le banc QEMU juge de l'exterieur (trace QEMU, empreintes) et a son
     temoin.

Fail-closed ; chaque regle a son temoin negatif (une mutation des sources
qui doit faire rougir le garde).
"""
from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
ATA = "src/drivers/block/ata.rs"
NVME = "src/drivers/block/nvme.rs"
XHCI = "src/drivers/usb/xhci_active.rs"
BBX = "src/drivers/usb/blackbox_storage.rs"
SONDE = "src/platform/pc/sonde_ecriture_interne.rs"
PCMOD = "src/platform/pc/mod.rs"
CARGO = "Cargo.toml"
BANC = "tools/ci/run_cle_usb_test.sh"
FICHIERS = (ATA, NVME, XHCI, BBX, SONDE, PCMOD, CARGO, BANC)


def sans_commentaires(t: str) -> str:
    return "\n".join(l.split("//", 1)[0] for l in t.splitlines())


def corps(texte: str, signature: str) -> str | None:
    """Le corps de la fonction dont la ligne de signature contient `signature`."""
    i = texte.find(signature)
    if i < 0:
        return None
    j = texte.find("{", i)
    profondeur = 0
    for k in range(j, len(texte)):
        if texte[k] == "{":
            profondeur += 1
        elif texte[k] == "}":
            profondeur -= 1
            if profondeur == 0:
                return texte[j : k + 1]
    return None


def avant(corps_fn: str, garde: str, action: str) -> bool:
    g, a = corps_fn.find(garde), corps_fn.find(action)
    return 0 <= g < a


def verifie(racine: Path, tout_src: bool = True) -> list[str]:
    fautes: list[str] = []
    try:
        t = {f: (racine / f).read_text(encoding="utf-8") for f in FICHIERS}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    s = {f: sans_commentaires(v) for f, v in t.items()}

    # 1. ATA
    sources = list((racine / "src").rglob("*.rs")) if tout_src else [racine / f for f in FICHIERS if f.endswith(".rs")]
    commandes = 0
    for f in sources:
        rel = f.relative_to(racine).as_posix()
        texte = sans_commentaires(f.read_text(encoding="utf-8", errors="replace"))
        n = texte.count("outb(COMMAND")
        commandes += n
        if n and rel != ATA:
            fautes.append(f"{rel} : ecrit le registre de commande ATA hors de {ATA}")
        if re.search(r"outb\(\s*0x(1[fF]7|177)\b", texte):
            fautes.append(f"{rel} : ecrit un port de commande ATA en dur")
    if commandes != 1:
        fautes.append(f"`outb(COMMAND` apparait {commandes} fois (attendu : 1, dans emet_commande)")
    emet = corps(s[ATA], "unsafe fn emet_commande(")
    if not emet or not avant(emet, "ecriture_interne::ata(", "outb(COMMAND"):
        fautes.append(f"{ATA} : emet_commande n'interroge pas la regle avant d'ecrire la commande")

    # 2-3. NVMe
    if s[NVME].count("write_volatile(emplacement") != 1:
        fautes.append(f"{NVME} : entree de soumission ecrite hors de File::pose")
    pose = corps(s[NVME], "unsafe fn pose(&mut self, sqe: &Sqe)")
    if not pose or not avant(pose, "ecriture_interne::nvme(self.admin, sqe)", "write_volatile(emplacement"):
        fautes.append(f"{NVME} : File::pose n'interroge pas la regle avant de poser l'entree")
    if "admin: file == 0," not in s[NVME]:
        fautes.append(f"{NVME} : `admin` n'est pas deduit de l'identifiant de la file")
    # `sq.pose(` : les deux appelants ; `soumission.pose()` est l'anneau.
    appels = re.findall(r"\bsq\.pose\(", s[NVME])
    if len(appels) != 2:
        fautes.append(f"{NVME} : File::pose appelee {len(appels)} fois (attendu : emet et soumet_es)")
    emet_es = corps(s[NVME], "fn emet_es(")
    if not emet_es or "jamais_pose(cid)" not in emet_es:
        fautes.append(f"{NVME} : un echec de soumission ne rend pas l'identifiant (jamais_pose)")

    # 4. USB
    cbw = 0
    for f in sources:
        rel = f.relative_to(racine).as_posix()
        n = sans_commentaires(f.read_text(encoding="utf-8", errors="replace")).count("encode_cbw(")
        if n and rel not in (XHCI, BBX, "src/drivers/usb/stockage/decodage.rs"):
            fautes.append(f"{rel} : code un CBW hors des deux transports")
        cbw += n if rel in (XHCI, BBX) else 0
    if cbw != 2:
        fautes.append(f"`encode_cbw(` appele {cbw} fois (attendu : 2)")
    une = corps(s[XHCI], "fn bot_commande_une_fois(")
    if not une or not avant(une, "ecriture_interne::scsi(", "encode_cbw("):
        fautes.append(f"{XHCI} : bot_commande_une_fois code le CBW sans interroger la regle")
    bb = corps(s[BBX], "fn blackbox_bot_en_place(")
    if not bb or not avant(bb, "ecriture_interne::scsi(", "encode_cbw("):
        fautes.append(f"{BBX} : la boite noire code le CBW sans interroger la regle")
    dem = corps(s[XHCI], "fn demarre_stockage(")
    if not dem or not avant(dem, "etablit_identite_stockage(", "stockage_vidange("):
        fautes.append(f"{XHCI} : l'identite n'est pas etablie avant la sonde de vidange")
    part = corps(s[BBX], "fn blackbox_find_partition(")
    if not part or "etablit_identite(" not in part:
        fautes.append(f"{BBX} : la boite noire n'etablit pas l'identite du support")

    # 5. Options et sonde
    m = re.search(r"^default\s*=\s*\[([^\]]*)\]", s[CARGO], re.M)
    if not m or re.search(r"cle-usb-test|banc-ecriture-interne", m.group(1)):
        fautes.append(f"{CARGO} : cle-usb-test ou banc-ecriture-interne dans default")
    for option in ("cle-usb-test = []", "banc-ecriture-interne = []"):
        if option not in s[CARGO]:
            fautes.append(f"{CARGO} : option `{option}` absente")
    if not re.search(r'#\[cfg\(feature = "banc-ecriture-interne"\)\]\s*pub mod sonde_ecriture_interne;', s[PCMOD]):
        fautes.append(f"{PCMOD} : la sonde n'est pas derriere banc-ecriture-interne")
    cmd = corps(s[SONDE], "pub fn commande()")
    if not cmd or not avant(cmd, "if !sous_qemu()", "ata::write("):
        fautes.append(f"{SONDE} : la sonde ne refuse pas de tourner hors d'une VM QEMU")
    for f, fn in ((ATA, "pub fn sonde_brute("), (NVME, "pub fn sonde_brute("), (XHCI, "pub fn sonde_scsi(")):
        i = s[f].find(fn)
        if i < 0 or '#[cfg(feature = "banc-ecriture-interne")]' not in s[f][max(0, i - 120) : i]:
            fautes.append(f"{f} : `{fn}` n'est pas derriere banc-ecriture-interne")

    # 6. Banc
    for motif in ("-trace events=", "ide_bus_exec_cmd", "pci_nvme_io_cmd", "scsi_req_parsed",
                  "BOOT_TEMOIN", "le banc est aveugle", "sha256sum"):
        if motif not in t[BANC]:
            fautes.append(f"{BANC} : `{motif}` absent (juge exterieur ou temoin)")
    return fautes


NEGATIFS = [
    # 1. ATA
    (ATA, "    if !crate::drivers::ecriture_interne::ata(commande, lba) {\n        return false;\n    }\n", ""),
    (ATA, "fn flush(drive: Drive) -> bool {", "fn flush_contourne(drive: Drive) { unsafe { outb(COMMAND, 0xE7) } }\nfn flush(drive: Drive) -> bool {"),
    (SONDE, "fn sous_qemu() -> bool {", "fn port() { unsafe { crate::arch::x86_64::ports::outb(0x1F7, 0x30) } }\nfn sous_qemu() -> bool {"),
    # 2-3. NVMe
    (NVME, "        if !crate::drivers::ecriture_interne::nvme(self.admin, sqe) {\n            return Err(\"refus-cle-usb-test\");\n        }\n", ""),
    (NVME, "        admin: file == 0,", "        admin: true,"),
    (NVME, "        cid_jamais_pose(cid);\n        return Err(raison);", "        return Err(raison);"),
    (NVME, "/// Pose une commande d'entree-sortie et sonne.", "fn contourne(sq: &mut File, s: &Sqe) { unsafe { let _ = sq.pose(s); } }\n/// Pose une commande d'entree-sortie et sonne."),
    # 4. USB
    (XHCI, "    if !crate::drivers::ecriture_interne::scsi(controller.stockages[index].identite, cdb) {\n        return Err(REFUS_CLE_USB_TEST);\n    }\n", ""),
    (BBX, "    if !crate::drivers::ecriture_interne::scsi(storage.identite, cdb) {\n        return Err(REFUS_CLE_USB_TEST);\n    }\n", ""),
    (XHCI, "    etablit_identite_stockage(controller, device, index);\n    let sonde = stockage_vidange(", "    let sonde = stockage_vidange("),
    (SONDE, "fn sous_qemu() -> bool {", "fn cbw(c: &mut [u8; 31]) { crate::drivers::xhci_active::stockage::encode_cbw(c, 0, 0, false, 0, &[0x2A]); }\nfn sous_qemu() -> bool {"),
    # 5. Options et sonde
    (CARGO, 'default = ["legacy-boot"]', 'default = ["legacy-boot", "cle-usb-test"]'),
    (PCMOD, '#[cfg(feature = "banc-ecriture-interne")]\npub mod sonde_ecriture_interne;', "pub mod sonde_ecriture_interne;"),
    (SONDE, "    if !sous_qemu() {", "    if false && !sous_qemu() {"),
    # 6. Banc
    (BANC, "-trace events=", "-no-trace="),
    (BANC, "le banc est aveugle", "rien"),
]


def temoins() -> str | None:
    for n, (f, a, b) in enumerate(NEGATIFS, 1):
        with tempfile.TemporaryDirectory() as d:
            racine = Path(d)
            for rel in FICHIERS:
                (racine / rel).parent.mkdir(parents=True, exist_ok=True)
                (racine / rel).write_text((RACINE / rel).read_text(encoding="utf-8"), encoding="utf-8")
            texte = (racine / f).read_text(encoding="utf-8")
            if a not in texte:
                return f"temoin {n} : motif introuvable dans {f}"
            (racine / f).write_text(texte.replace(a, b, 1), encoding="utf-8")
            if not verifie(racine, tout_src=False):
                return f"temoin negatif {n} ne rougit pas -- garde inoperant"
    return None


def main() -> int:
    fautes = verifie(RACINE)
    for f in fautes:
        print("ecriture interne :", f)
    if fautes:
        return 1
    t = temoins()
    if t:
        print("ecriture interne :", t)
        return 1
    print(f"ECRITURE_INTERNE_OK negatifs={len(NEGATIFS)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
