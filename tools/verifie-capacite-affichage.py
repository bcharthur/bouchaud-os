#!/usr/bin/env python3
"""BOUCHAUD_COMPOSITORD_PRESENTATION_V1 : la capacite d'affichage n'a qu'une
origine, et `bo_display_present` juge avant de lire.

  1. `Object::Display` n'est construit QUE dans `src/kernel/native/affichage.rs`
     (aucun appel systeme ne fabrique une capacite d'affichage) ;
  2. la capacite donnee au compositeur ne porte ni DUP ni TRANSFER ;
  3. `present` exige DISPLAY_PRESENT sur un objet Display, puis READ sur la
     region, puis `contrat::valide`, et ce dans cet ordre, AVANT la premiere
     lecture de la region (`region.read`) ;
  4. le bureau noyau declare qu'il tient l'ecran (`bureau_noyau_demarre`)
     avant de lancer son fil ;
  5. le banc QEMU decode la capture au pixel.

Fail-closed ; chaque regle a son temoin negatif.
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
AFF = "src/kernel/native/affichage.rs"
WM = "src/gui/window_manager.rs"
BANC = "tools/ci/run_native_ipc_probe.sh"


def sans_commentaires(t):
    return "\n".join(l.split("//", 1)[0] for l in t.splitlines())


def verifie(racine: Path) -> list:
    fautes = []
    try:
        aff = sans_commentaires((racine / AFF).read_text(encoding="utf-8"))
        wm = sans_commentaires((racine / WM).read_text(encoding="utf-8"))
        banc = (racine / BANC).read_text(encoding="utf-8")
    except OSError as e:
        return [f"lecture impossible : {e}"]
    for f in (racine / "src").rglob("*.rs"):
        rel = f.relative_to(racine).as_posix()
        if rel in (AFF, "src/kernel/native/object.rs"):
            continue
        if re.search(r"Object::Display\b", sans_commentaires(f.read_text(encoding="utf-8", errors="replace"))):
            fautes.append(f"{rel} : construit ou filtre Object::Display hors de {AFF}")
    m = re.search(r"DROITS_COMPOSITEUR: Rights = Rights\(([^;]*)\);", aff)
    if not m:
        fautes.append(f"{AFF} : DROITS_COMPOSITEUR introuvable")
    elif re.search(r"\b(DUP|TRANSFER)\b", m.group(1)):
        fautes.append(f"{AFF} : la capacite du compositeur peut se dupliquer ou se transferer")
    corps = aff[aff.find("pub fn present("):]
    jalons = ["Rights::DISPLAY_PRESENT, ObjectKind::Display", "Rights::READ, ObjectKind::SharedRegion",
              "contrat::valide(", "region.read("]
    pos = [corps.find(j) for j in jalons]
    if any(p < 0 for p in pos) or pos != sorted(pos):
        fautes.append(f"{AFF} : present ne verifie plus capacite, droit de lecture et bornes AVANT de lire")
    i = wm.find("pub fn run()")
    run = wm[i:wm.find("}", i)]
    if "affichage::bureau_noyau_demarre()" not in run or run.find("bureau_noyau_demarre") > run.find("run_noyau"):
        fautes.append(f"{WM} : le bureau noyau ne declare plus tenir l'ecran avant son fil")
    if "verifie_capture_composited.py" not in banc:
        fautes.append(f"{BANC} : la capture n'est plus decodee au pixel")
    return fautes


def temoins():
    negatifs = [
        (AFF, "DROITS_COMPOSITEUR: Rights = Rights(Rights::DISPLAY_PRESENT.0 | Rights::INSPECT.0);",
         "DROITS_COMPOSITEUR: Rights = Rights(Rights::DISPLAY_PRESENT.0 | Rights::INSPECT.0 | Rights::DUP.0);"),
        (AFF, "    contrat::valide(&d, region.len(), el, eh)", "    let _ = (&d, el, eh); Ok::<(), contrat::Refus>(())"),
        (AFF, "Rights::DISPLAY_PRESENT, ObjectKind::Display)", "Rights::NONE, ObjectKind::Display)"),
        (WM, "    crate::kernel::native::affichage::bureau_noyau_demarre();\n", ""),
        (BANC, "verifie_capture_composited.py", "verifie_rien.py"),
        ("src/kernel/native/abi/dispatch.rs", "        numbers::DISPLAY_PRESENT =>",
         "        numbers::HANDLE_COUNT if false => Ok(handle::install(Object::Display, Rights::TOUS)?.raw()),\n        numbers::DISPLAY_PRESENT =>"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        with tempfile.TemporaryDirectory() as d:
            racine = Path(d)
            for rel in (AFF, WM, BANC, "src/kernel/native/abi/dispatch.rs", "src/kernel/native/object.rs"):
                (racine / rel).parent.mkdir(parents=True, exist_ok=True)
                (racine / rel).write_text((RACINE / rel).read_text(encoding="utf-8"), encoding="utf-8")
            t = (racine / f).read_text(encoding="utf-8")
            if a not in t:
                return f"temoin {n} : motif introuvable dans {f}"
            (racine / f).write_text(t.replace(a, b, 1), encoding="utf-8")
            if not verifie(racine):
                return f"temoin negatif {n} ne rougit pas -- garde inoperant"
    return None


def main():
    fautes = verifie(RACINE)
    for f in fautes:
        print("capacite d'affichage :", f)
    if fautes:
        return 1
    t = temoins()
    if t:
        print("capacite d'affichage :", t)
        return 1
    print("CAPACITE_AFFICHAGE_OK negatifs=6")
    return 0


if __name__ == "__main__":
    sys.exit(main())
