#!/usr/bin/env python3
"""Le fil `usb-hid` ne scrute a la milliseconde que s'il sert un HID
(P13_HID_SANS_PERIPHERIQUE_V1).

Le fil est Interactive, sensible a la latence et epingle au coeur zero. Sans
aucun HID il se reveillait quand meme mille fois par seconde ; sous le
reveil precis, chaque sommeil d'un tick armait un tir et une preemption
ciblee sur le coeur zero occupe.

Regles :

  1. `fil_hid` calcule ses ticks par `ticks_apres_tour_hid(hid_polling())` ;
  2. l'echeance chronometree (`echeance_pour`) et le sommeil (`sleep_ticks`)
     utilisent les MEMES ticks : le chronometre ne doit pas accuser
     l'ordonnanceur d'un retard qu'il a demande ;
  3. la decision pure garde 1 tick quand un HID est servi, et une pause
     d'oisivete superieure au tick sinon (test hote `test_xhci_policy.rs`).

Fail-closed ; tests negatifs par mutation.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
USB = RACINE / "src/drivers/usb/xhci_active.rs"
PUR = RACINE / "src/drivers/usb/xhci_policy.rs"
TEST = RACINE / "tools/platform/test_xhci_policy.rs"
CHOIX = "let ticks = politique_xhci::ticks_apres_tour_hid(hid_polling());"


def corps(source: str, signature: str) -> str:
    debut = source.find(signature)
    if debut < 0:
        return ""
    ouverture = source.find("{", debut)
    profondeur = 0
    for i in range(ouverture, len(source)):
        if source[i] == "{":
            profondeur += 1
        elif source[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[ouverture:i + 1]
    return ""


def verifie(usb: str, pur: str, test: str) -> list:
    fautes = []
    fil = corps(usb, "fn fil_hid() -> ! {")
    choix = fil.find(CHOIX)
    chrono = fil.find("echeance = crate::kernel::task::echeance_pour(ticks);")
    sommeil = fil.find("crate::kernel::task::sleep_ticks(ticks);")
    if not 0 <= choix < chrono < sommeil:
        fautes.append("fil_hid : cadence non conditionnee au HID servi, ou chrono et sommeil divergent")
    if "sleep_ticks(1)" in fil or "echeance_pour(1)" in fil:
        fautes.append("fil_hid : la milliseconde est de nouveau inconditionnelle")
    if "if hid_servi { 1 } else { PAUSE_HID_OISIF_TICKS }" not in corps(pur, "pub const fn ticks_apres_tour_hid("):
        fautes.append("xhci_policy.rs : la cadence n'est plus celle testee")
    if "fn le_fil_hid_garde_la_milliseconde_seulement_quand_un_hid_est_servi()" not in test:
        fautes.append("test hote absent : cadence du fil HID")
    return fautes


def negatifs(usb: str, pur: str, test: str) -> list:
    mutations = [
        ("usb", CHOIX, "let ticks = 1;"),
        ("usb", "echeance = crate::kernel::task::echeance_pour(ticks);", "echeance = crate::kernel::task::echeance_pour(1);"),
        ("usb", "crate::kernel::task::sleep_ticks(ticks);", "crate::kernel::task::sleep_ticks(1);"),
        ("pur", "if hid_servi { 1 } else { PAUSE_HID_OISIF_TICKS }", "if hid_servi { 1 } else { 1 }"),
        ("test", "fn le_fil_hid_garde_la_milliseconde_seulement_quand_un_hid_est_servi()", "fn x()"),
    ]
    echecs = []
    for cle, avant, apres in mutations:
        t = {"usb": usb, "pur": pur, "test": test}
        if avant not in t[cle]:
            echecs.append(f"mutation inapplicable : {avant[:40]}")
            continue
        t[cle] = t[cle].replace(avant, apres, 1)
        if not verifie(t["usb"], t["pur"], t["test"]):
            echecs.append(f"mutation non detectee : {avant[:40]}")
    return echecs


def main() -> int:
    usb, pur, test = (p.read_text(encoding="utf-8") for p in (USB, PUR, TEST))
    fautes = verifie(usb, pur, test) + negatifs(usb, pur, test)
    for faute in fautes:
        print(f"ECHEC {faute}")
    if fautes:
        return 1
    print("fil usb-hid sans peripherique : ok (5 mutations detectees)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
