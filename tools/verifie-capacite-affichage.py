#!/usr/bin/env python3
"""BOUCHAUD_COMPOSITORD_PRESENTATION_V1 : la capacite d'affichage n'a qu'une
origine, et `bo_display_present` juge avant de lire.

  1. `Object::Display` n'est construit QUE dans `src/kernel/native/affichage.rs`
     (aucun appel systeme ne fabrique une capacite d'affichage) ;
  2. la capacite donnee au compositeur ne porte ni DUP ni TRANSFER ;
  3. `present` exige DISPLAY_PRESENT sur un objet Display, puis READ sur la
     region, puis `contrat::valide`, et ce dans cet ordre, AVANT la premiere
     lecture de la region (`region.read`) ;
  4. le bureau noyau DEMANDE l'ecran (`bureau_noyau_demarre`) avant de lancer
     son fil, et NE LE LANCE PAS si la demande est refusee ; la demande publie
     sa prise puis lit celle du compositeur ring 3 (bureau configure ou ecran
     deja pris), et `present` publie la sienne puis relit celle du bureau
     noyau (BOUCHAUD_BUREAU_RING3_2A : jamais deux compositeurs) ;
  5. le banc QEMU decode la capture au pixel ;
  6. `Object::Fenetre` (BOUCHAUD_BUREAU_RING3_2A) n'est construit qu'au
     lancement du compositeur, et sa capacite ne porte ni DUP ni TRANSFER.

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
FEN = "src/kernel/native/fenetre.rs"


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
        texte = sans_commentaires(f.read_text(encoding="utf-8", errors="replace"))
        if re.search(r"Object::Display\b", texte):
            fautes.append(f"{rel} : construit ou filtre Object::Display hors de {AFF}")
        if rel != FEN and re.search(r"Object::Fenetre\(", texte):
            fautes.append(f"{rel} : construit ou filtre Object::Fenetre hors de {AFF}")
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
    run = wm[i:wm.find("\n}\n", i)]
    refus = re.search(r"if !crate::kernel::native::affichage::bureau_noyau_demarre\(\) \{\s*return;\s*\}", run)
    if not refus or run.find("run_noyau") < 0 or refus.start() > run.find("run_noyau"):
        fautes.append(f"{WM} : le bureau noyau ne demande plus l'ecran, ou demarre malgre un refus, avant son fil")
    dem = aff[aff.find("pub fn bureau_noyau_demarre("):]
    dem = dem[:dem.find("\n}\n")]
    pub = dem.find("BUREAU_NOYAU.store(true, Ordering::SeqCst)")
    lit = dem.find("BUREAU_RING3.load(Ordering::SeqCst)")
    lit2 = dem.find("ECRAN_AU_COMPOSITEUR.load(Ordering::SeqCst)")
    if pub < 0 or lit < pub or lit2 < pub or "return false;" not in dem:
        fautes.append(f"{AFF} : la demande du bureau noyau ne publie plus sa prise avant de lire celle du compositeur")
    pres = aff[aff.find("pub fn present("):]
    prise = pres.find("ECRAN_AU_COMPOSITEUR.store(true, Ordering::SeqCst)")
    relit = pres.find("BUREAU_NOYAU.load(Ordering::SeqCst)", max(prise, 0))
    pose = pres.find("region.read(")
    if prise < 0 or relit < 0 or not (prise < relit < pose):
        fautes.append(f"{AFF} : present ne relit plus la prise du bureau noyau apres avoir publie la sienne")
    lance = aff[aff.find("pub fn lance_compositeur("):]
    lance = lance[:lance.find("\n}\n")]
    echec = lance[lance.find("BOUCHAUD_COMPOSITEUR_ECHEC") - 400:lance.find("BOUCHAUD_COMPOSITEUR_ECHEC")]
    if "BUREAU_RING3.store(false" not in echec:
        fautes.append(f"{AFF} : un compositeur qui ne demarre pas laisse l'ecran reserve (revue #295)")
    info = aff[aff.find("pub fn info("):]
    info = info[:info.find("\n}\n")]
    if "Rights::DISPLAY_PRESENT, ObjectKind::Display)" not in info:
        fautes.append(f"{AFF} : info ne verifie plus la capacite d'affichage")
    fen = sans_commentaires((racine / FEN).read_text(encoding="utf-8"))
    m = re.search(r"DROITS_COMPOSITEUR: Rights = Rights\(([^;]*)\);", fen)
    if not m or re.search(r"\b(DUP|TRANSFER)\b", m.group(1)):
        fautes.append(f"{FEN} : la capacite fenetre peut se dupliquer ou se transferer")
    if "verifie_capture_composited.py" not in banc:
        fautes.append(f"{BANC} : la capture n'est plus decodee au pixel")
    return fautes


def temoins():
    negatifs = [
        (AFF, "DROITS_COMPOSITEUR: Rights = Rights(Rights::DISPLAY_PRESENT.0 | Rights::INSPECT.0);",
         "DROITS_COMPOSITEUR: Rights = Rights(Rights::DISPLAY_PRESENT.0 | Rights::INSPECT.0 | Rights::DUP.0);"),
        (AFF, "    contrat::valide(&d, region.len(), el, eh)", "    let _ = (&d, el, eh); Ok::<(), contrat::Refus>(())"),
        (AFF, "table.lookup_kind(HandleId::from_raw(args[0]), Rights::DISPLAY_PRESENT, ObjectKind::Display)",
         "table.lookup_kind(HandleId::from_raw(args[0]), Rights::NONE, ObjectKind::Display)"),
        (AFF, ".lookup_kind(HandleId::from_raw(args[0]), Rights::DISPLAY_PRESENT, ObjectKind::Display)\n        .map_err(|e| refuse(e, \"capacite\"))?;\n    if BUREAU_NOYAU",
         ".lookup_kind(HandleId::from_raw(args[0]), Rights::INSPECT, ObjectKind::Display)\n        .map_err(|e| refuse(e, \"capacite\"))?;\n    if BUREAU_NOYAU"),
        (WM, "    if !crate::kernel::native::affichage::bureau_noyau_demarre() {\n        return;\n    }\n", ""),
        (WM, "affichage::bureau_noyau_demarre() {\n        return;\n    }", "affichage::bureau_noyau_demarre() {\n    }"),
        (AFF, "    if BUREAU_RING3.load(Ordering::SeqCst) || ECRAN_AU_COMPOSITEUR.load(Ordering::SeqCst) {",
         "    if false {"),
        (AFF, "    if BUREAU_NOYAU.load(Ordering::SeqCst) {\n        return Err(refuse(Error::WouldBlock, \"bureau-noyau\"));\n    }\n    // 5.",
         "    // 5."),
        (AFF, "            BUREAU_RING3.store(false, Ordering::SeqCst);\n            drop(fenetres.take());", "            drop(fenetres.take());"),
        (FEN, "pub const DROITS_COMPOSITEUR: Rights = Rights(Rights::READ.0 | Rights::WRITE.0 | Rights::WAIT.0 | Rights::INSPECT.0);",
         "pub const DROITS_COMPOSITEUR: Rights = Rights(Rights::READ.0 | Rights::WRITE.0 | Rights::WAIT.0 | Rights::TRANSFER.0);"),
        ("src/kernel/native/abi/dispatch.rs", "        numbers::FENETRE_LIT =>",
         "        numbers::HANDLE_COUNT if false => { let _ = Object::Fenetre(todo!()); Ok(0) }\n        numbers::FENETRE_LIT =>"),
        (BANC, "verifie_capture_composited.py", "verifie_rien.py"),
        ("src/kernel/native/abi/dispatch.rs", "        numbers::DISPLAY_PRESENT =>",
         "        numbers::HANDLE_COUNT if false => Ok(handle::install(Object::Display, Rights::TOUS)?.raw()),\n        numbers::DISPLAY_PRESENT =>"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        with tempfile.TemporaryDirectory() as d:
            racine = Path(d)
            for rel in (AFF, WM, BANC, FEN, "src/kernel/native/abi/dispatch.rs", "src/kernel/native/object.rs"):
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
    print("CAPACITE_AFFICHAGE_OK negatifs=13")
    return 0


if __name__ == "__main__":
    sys.exit(main())
