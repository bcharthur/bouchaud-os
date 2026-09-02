#!/usr/bin/env python3
"""Le registre des taches ne doit pas redevenir un contournement de l'emprunt.

# Ce que ce garde-fou protege

La premiere version du registre rendait `&'static mut Task` depuis la MEME
lecture partagee que `&Task`, et recyclait un emplacement en ecrasant son
contenu sur place. Garder l'adresse reglait la duree de vie, et rien d'autre :
une adresse stable n'est pas une identite stable.

Aucune de ces deux fautes ne fait echouer quoi que ce soit. Le noyau compile,
demarre, et fonctionne -- jusqu'a ce qu'un emplacement soit recycle sous un
lecteur. C'est pour cela qu'elles ont besoin d'un garde-fou plutot que d'un
test.

# Les quatre regles

  1. Aucune fonction du registre ne rend `&'static mut Task`. L'acces exclusif
     passe par un garde, et le garde par un drapeau pris en exclusion mutuelle.
  2. Chaque emplacement porte une GENERATION, incrementee a chaque
     installation. C'est ce qui rend un ancien handle detectable.
  3. Toute reference porte un garde de lecture ; la lecture par identite
     compare la generation sous ce garde.
  4. Le recyclage prend l'ecriture, attend tous les lecteurs et incremente la
     generation AVANT d'ecrire.

Et une cinquieme, ailleurs : la file d'execution porte des identites, pas des
indices. Un indice conserve par une file designerait la tache suivante apres
recyclage -- c'est l'ABA, et il se voit comme une tache fantome qui prend des
quantums.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
REGISTRE = RACINE / "src" / "kernel" / "process" / "thread" / "registre.rs"
CPU_LOCAL = RACINE / "src" / "arch" / "x86_64" / "cpu_local.rs"
ORDO = RACINE / "src" / "kernel" / "process" / "thread" / "ordonnancement.rs"
COURANT = RACINE / "src" / "kernel" / "process" / "thread" / "courant.rs"


def sans_commentaires(source: str) -> str:
    """Le code seul : les commentaires citent l'ancienne forme pour l'expliquer."""
    return re.sub(r"//[^\n]*", "", source)


def corps(source: str, signature: str) -> str:
    debut = source.index(signature)
    profondeur = 0
    for position in range(debut, len(source)):
        if source[position] == "{":
            profondeur += 1
        elif source[position] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[debut : position + 1]
    raise SystemExit(f"corps introuvable : {signature}")


def main() -> int:
    if not REGISTRE.exists():
        print(f"introuvable : {REGISTRE}")
        return 1
    code = sans_commentaires(REGISTRE.read_text(encoding="utf-8"))
    fautes = []

    # --- 1. aucun `&mut` depuis une lecture partagee ------------------------
    for trouve in re.finditer(r"pub fn (\w+)\([^)]*\)\s*->\s*([^{\n]+)", code):
        nom, retour = trouve.group(1), trouve.group(2)
        if "&'static mut Task" in retour or "&mut Task" in retour:
            fautes.append(
                f"  registre.rs  `{nom}` rend `{retour.strip()}` : un `&mut` "
                f"promet une exclusivite que le registre ne fait pas respecter"
            )
    if "GardeTache" not in code:
        fautes.append(
            "  registre.rs  le garde d'acces exclusif a disparu : le contenu "
            "non atomique n'a plus de mecanisme d'exclusion"
        )
    if "compare_exchange(false, true" not in code:
        fautes.append(
            "  registre.rs  l'exclusivite n'est plus prise en exclusion "
            "mutuelle : deux gardes pourraient coexister"
        )
    if re.search(r"pub fn registre_tache(?:_id)?\([^)]*\)\s*->\s*Option<&'static Task>", code):
        fautes.append(
            "  registre.rs  une lecture rend encore `&'static Task` sans garde : "
            "la generation ne protege pas une reference deja obtenue"
        )

    # --- 2. la generation existe et s'incremente ----------------------------
    if "generation" not in code:
        fautes.append(
            "  registre.rs  les emplacements n'ont plus de generation : un "
            "ancien handle redevient indetectable apres recyclage"
        )
    if "struct TacheId" not in code or "pub const fn generation" not in code:
        fautes.append(
            "  registre.rs  `TacheId` ne porte plus sa generation : un indice "
            "nu ne distingue pas deux incarnations du meme emplacement"
        )

    # --- 3. la lecture par identite compare la generation -------------------
    try:
        lecture = corps(code, "pub fn registre_tache_id(")
    except SystemExit:
        lecture = ""
        fautes.append("  registre.rs  `registre_tache_id` a disparu")
    if lecture and "generation" not in lecture:
        fautes.append(
            "  registre.rs  `registre_tache_id` ne compare plus la generation : "
            "il rendrait la nouvelle incarnation a un ancien handle"
        )
    if lecture and not (
        "RegistreLecture::acquire()" in lecture
        and lecture.find("RegistreLecture::acquire()") < lecture.find("generation")
    ):
        fautes.append(
            "  registre_tache_id  compare la generation hors d'un garde de "
            "lecture : le recycleur peut encore ecraser la reference rendue"
        )

    if "struct GardeLectureTache" not in code or "lecture: RegistreLecture" not in code:
        fautes.append(
            "  registre.rs  la reference partagee ne conserve plus son garde "
            "de lecture jusqu'au Drop"
        )

    try:
        ecriture = corps(code, "fn acquire() -> Self {")
    except SystemExit:
        ecriture = ""
    # Le fichier contient deux `acquire`; cibler explicitement le bloc ecrivain.
    debut_ecrivain = code.find("impl RegistreEcriture")
    ecriture = corps(code[debut_ecrivain:], "fn acquire() -> Self {") if debut_ecrivain >= 0 else ""
    if not ecriture:
        fautes.append("  registre.rs  le garde d'ecriture du recycleur a disparu")
    else:
        masque = ecriture.find("interrupts::disable()")
        publie = ecriture.find("compare_exchange(false, true")
        attend = ecriture.find("LECTEURS.load(Ordering::Acquire) != 0")
        if not (0 <= masque < publie < attend):
            fautes.append(
                "  RegistreEcriture  ordre invalide : masquer IRQ -> publier "
                "ecrivain -> attendre lecteurs est obligatoire"
            )

    # --- 4. le recyclage : ecriture, puis generation, puis remplacement -----
    try:
        ajoute = corps(code, "pub fn registre_ajoute(")
    except SystemExit:
        ajoute = ""
        fautes.append("  registre.rs  `registre_ajoute` a disparu")
    if ajoute:
        exclusivite = ajoute.find("RegistreEcriture::acquire()")
        generation = ajoute.find("prochaine_generation(")
        ecriture = ajoute.find("*ancienne =")
        if exclusivite == -1:
            fautes.append(
                "  registre_ajoute  recycle sans garde d'ecriture : une "
                "reference partagee peut voir son contenu remplace sous elle"
            )
        elif generation == -1:
            fautes.append("  registre_ajoute  n'incremente plus la generation")
        elif ecriture != -1 and not (exclusivite < generation < ecriture):
            fautes.append(
                "  registre_ajoute  l'ordre ecriture -> generation -> "
                "remplacement n'est pas respecte : c'est cet ordre qui empeche "
                "d'observer un contenu a moitie remplace"
            )

    if COURANT.exists():
        courant = sans_commentaires(COURANT.read_text(encoding="utf-8"))
        try:
            vue = corps(courant, "pub struct VueRegistre")
        except SystemExit:
            vue = ""
        if "RegistreLecture" not in vue:
            fautes.append(
                "  courant.rs  VueRegistre ne conserve plus le garde de lecture "
                "pendant ses indexations et iterations"
            )

    # --- 5. la file d'execution porte des identites -------------------------
    if CPU_LOCAL.exists():
        cpu_local = sans_commentaires(CPU_LOCAL.read_text(encoding="utf-8"))
        if re.search(r"run_queue:\s*SpinLockIrq<Vec<usize>>", cpu_local):
            fautes.append(
                "  cpu_local.rs  la file d'execution porte des INDICES : apres "
                "recyclage, une entree laissee par une tache morte ordonnance "
                "la suivante, qui n'a rien demande"
            )
    if ORDO.exists():
        ordo = sans_commentaires(ORDO.read_text(encoding="utf-8"))
        if "registre_tache_id(" not in ordo:
            fautes.append(
                "  ordonnancement.rs  le consommateur de la file ne verifie "
                "plus la generation des entrees"
            )

    if fautes:
        print("registre des taches : regle violee")
        print("\n".join(fautes))
        return 1

    print(
        "ok  src/kernel/process/thread/registre.rs : aucun `&mut` partage, "
        "generations par emplacement, lecteurs quiescents avant recyclage, "
        "file generationnelle"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
