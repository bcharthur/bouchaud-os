#!/usr/bin/env python3
"""Les sondes du premier bouclage RTL8168 observent, et n'agissent pas.

BOUCHAUD_RTL8168_PREMIER_BOUCLAGE_V1

# Le fait

TRIGKEY, 04fef512, deux demarrages sur deux : premier tour parfait, puis plus
un descripteur rendu apres le bouclage 63 -> 0, pendant que `RxOK` monte.
Deux reprises de degre 0 sans effet, la troisieme (degre 3) fait repartir
l'anneau. Le releve ne dit pas ou vont les trames que `RxOK` annonce.

# La regle

Les sondes qui le diront ne doivent PAS changer ce qu'elles mesurent :

  1. `releve()`, `capture_lab` et `sonde_hors_anneau` -- que l'auditeur et la
     telemetrie appellent sans le verrou de reception -- n'ecrivent dans aucun
     registre ni descripteur, et ne declenchent pas de releve DTCC ;
  2. le releve DTCC n'ecrit que `CounterAddr*`, moitie haute AVANT moitie
     basse (sequence de `rtl8169_do_counters`), et il est borne dans le temps ;
  3. l'auditeur n'appelle pas le releve DTCC ;
  4. l'ombre n'est lue que si `est_ram_connue` l'accepte, et la photographie
     est prise AVANT que la carte ne demarre ;
  5. chaque reprise laisse `RX_RECOVERY_BEGIN` AVANT d'agir, et chaque verdict
     `RX_RECOVERY_END`, effectif ou non -- le port serie n'existe pas sur la
     TRIGKEY, c'etait le seul endroit ou le degre s'ecrivait.

Fail-closed ; six tests negatifs.
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
PILOTE = "src/drivers/network/rtl8168.rs"
AUDITD = "src/kernel/debug/lab/auditd.rs"
FICHIERS = (PILOTE, AUDITD)
ECRITURES = ("write8(", "write16(", "write32(", "desc_write32(", "desc_write64(")


def sans_commentaires(texte: str) -> str:
    return "\n".join(l.split("//", 1)[0] for l in texte.splitlines())


def corps(texte: str, nom: str) -> str | None:
    m = re.search(r"\bfn\s+" + re.escape(nom) + r"\s*\(", texte)
    if not m:
        return None
    ouverture = texte.find("{", m.end())
    profondeur = 0
    for i in range(ouverture, len(texte)):
        if texte[i] == "{":
            profondeur += 1
        elif texte[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return texte[ouverture:i + 1]
    return None


def verifie(racine: Path) -> list[str]:
    try:
        pilote = sans_commentaires((racine / PILOTE).read_text(encoding="utf-8"))
        auditd = sans_commentaires((racine / AUDITD).read_text(encoding="utf-8"))
    except OSError as e:
        return [f"lecture impossible : {e}"]
    fautes = []
    for nom in ("releve", "capture_lab", "sonde_hors_anneau"):
        c = corps(pilote, nom)
        if c is None:
            fautes.append(f"{PILOTE} : fn {nom} introuvable")
            continue
        for e in ECRITURES:
            if e in c:
                fautes.append(f"{PILOTE} : fn {nom} ecrit (`{e}`) -- une sonde ne modifie pas son objet")
        if "releve_compteurs_materiel(" in c:
            fautes.append(f"{PILOTE} : fn {nom} declenche un releve DTCC (ecriture materielle)")
    dtcc = corps(pilote, "releve_compteurs_materiel")
    if dtcc is None:
        fautes.append(f"{PILOTE} : releve_compteurs_materiel introuvable")
    else:
        ecrits = re.findall(r"write(?:8|16|32)\((\w+)", dtcc)
        if not ecrits or any(r not in ("REG_DTCC_HIGH", "REG_DTCC_LOW") for r in ecrits):
            fautes.append(f"{PILOTE} : le releve DTCC ecrit ailleurs que dans CounterAddr* ({ecrits})")
        elif ecrits[0] != "REG_DTCC_HIGH":
            fautes.append(f"{PILOTE} : le releve DTCC n'ecrit plus la moitie haute en premier")
        if "DTCC_DELAI_NS" not in dtcc:
            fautes.append(f"{PILOTE} : l'attente du releve DTCC n'est plus bornee")
        if any(w in dtcc for w in ("desc_write32(", "desc_write64(")):
            fautes.append(f"{PILOTE} : le releve DTCC touche aux descripteurs")
    if "releve_compteurs_materiel" in auditd:
        fautes.append(f"{AUDITD} : l'auditeur declenche un releve DTCC")
    photo = corps(pilote, "photographie_hors_anneau") or ""
    if not re.search(r"est_ram_connue\(ombre", photo) or photo.find("est_ram_connue") > photo.find("OMBRE_P = ombre"):
        fautes.append(f"{PILOTE} : l'ombre peut etre lue sans que la carte memoire l'accepte")
    init = corps(pilote, "init_with_device") or ""
    p_photo, p_prog = init.find("photographie_hors_anneau()"), init.find("programme_le_materiel()")
    if p_photo < 0 or p_prog < 0 or p_photo > p_prog:
        fautes.append(f"{PILOTE} : la photographie hors anneau n'est plus prise avant le demarrage")
    repare = corps(pilote, "repare_reception") or ""
    p_debut, p_isr = repare.find("RX_RECOVERY_BEGIN"), repare.find("maintenance_isr()")
    if p_debut < 0 or p_isr < 0 or p_debut > p_isr:
        fautes.append(f"{PILOTE} : la reprise n'annonce plus son degre AVANT d'agir (RX_RECOVERY_BEGIN)")
    verdict = corps(pilote, "publie_le_verdict") or ""
    p_fin, p_eff = verdict.find("RX_RECOVERY_END"), verdict.find("if issue.effective")
    if p_fin < 0 or p_eff < 0 or p_fin > p_eff:
        fautes.append(f"{PILOTE} : un verdict effectif ne laisse plus RX_RECOVERY_END")
    return fautes


def mutation(fichier: str, avant: str, apres: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        copie = Path(tmp)
        for f in FICHIERS:
            dest = copie / f
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text((RACINE / f).read_text(encoding="utf-8"), encoding="utf-8")
        cible = copie / fichier
        texte = cible.read_text(encoding="utf-8")
        if avant not in texte:
            return False
        cible.write_text(texte.replace(avant, apres, 1), encoding="utf-8")
        return bool(verifie(copie))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("rtl8168 premier bouclage : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        (PILOTE, "        // 7. Le materiel ecrit-il HORS de notre anneau ? Lecture seule.\n        sonde_hors_anneau();",
         "        releve_compteurs_materiel(raison);\n        sonde_hors_anneau();"),
        (PILOTE, "    write32(REG_DTCC_HIGH, (TALLY_P >> 32) as u32);\n    write32(REG_DTCC_LOW, bas);",
         "    write32(REG_DTCC_LOW, bas);\n    write32(REG_DTCC_HIGH, (TALLY_P >> 32) as u32);"),
        (AUDITD, "        CAPTURES.fetch_add(1, Ordering::Relaxed);",
         "        CAPTURES.fetch_add(1, Ordering::Relaxed);\n        unsafe { crate::drivers::rtl8168::releve_compteurs_materiel(4) };"),
        (PILOTE, "    if haut != 0 && memory::est_ram_connue(ombre, (N_RX * DESC_SIZE) as u64) {",
         "    if haut != 0 {"),
        (PILOTE, "        lab::id::RX_RECOVERY_BEGIN,", "        lab::id::RX_PROGRES,"),
        (PILOTE, "        lab::id::RX_RECOVERY_END,", "        lab::id::RX_PROGRES,"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"rtl8168 premier bouclage : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"RTL8168_PREMIER_BOUCLAGE_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
