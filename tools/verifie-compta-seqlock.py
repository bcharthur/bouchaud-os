#!/usr/bin/env python3
"""La comptabilite CPU ne s'ecrit que sous sa sequence.

BOUCHAUD_COMPTA_SEQLOCK_V1

# Ce que ce garde-fou protege

Run 37589903681 (KVM) : panique noyau « task: runtime > fenetre tid=118
delta=35786689 window=33031990 ». `mesure_processus` lisait le curseur d'une
tache et ses compteurs a deux instants, pendant que le coeur de la tache les
mettait a jour : la tranche comptait deux fois.

Le correctif est un verrou de sequence : les lecteurs (`lecture_compta`,
`proc_cpu_cumul`) relisent tant qu'une ecriture est en cours. Il ne vaut que
si TOUTE ecriture passe par une section. Une seule ecriture oubliee -- un
nouveau chemin de commutation, un repli ajoute pour un diagnostic -- et la
lecture dechiree revient, sans que rien ne le dise avant la prochaine panique
sous KVM.

# Les regles

1. Une ecriture (`range`, `store`, `fetch_add`, `swap`, `echange`) des champs
   de la tache (`last_account_ns`, `user_cpu_ns`, `kernel_cpu_ns`, `cpu_ns[]`)
   ou du bloc par coeur (`COMPTA_USER_NS`, `COMPTA_NOYAU_NS`,
   `COMPTA_DEBUT_NS`, `COMPTA_EN_NOYAU`, `CUMUL_USER_NS`, `CUMUL_NOYAU_NS`) se
   trouve DANS la fermeture d'un appel `compta_section(...)`, ou dans
   `account_until` (corps d'un repli, appele seulement en section), ou dans
   `frontiere_compta_bloc` (appele seulement par `frontiere_compta`, qui tient
   `COMPTA_SEQ_CPU`).
2. `account_until(` n'est appele que dans une section ; `frontiere_compta_bloc(`
   seulement depuis `frontiere_compta`.
3. `compta_section` masque les interruptions et verifie la parite (un second
   ecrivain la casserait) ; `Task::new` initialise `compta_seq`.
4. `lecture_compta` ne borne pas ses relectures : une borne rendrait, au bout
   du compte, la lecture dechiree.
5. BOUCHAUD_COMPTA_INSTANT_EN_SECTION_V1 : l'instant passe a `account_until`
   est lu DANS la meme section (`let now = ...monotonic_ns()` entre
   l'ouverture de `compta_section(` et l'appel). Lu avant, un vCPU suspendu
   entre la lecture et la section laissait un lecteur compter la tranche
   vive jusqu'a un instant plus tardif que le repli : `utime` reculait
   (compta-stress, TCG SMP4, 1 passe sur 8).
6. BOUCHAUD_COMPTA_REARME_EN_SECTION_V1 : `rearme_compta_apres_idle` lit son
   instant ET son coeur DANS la section, et n'y touche pas a une tranche deja
   armee. Au retour du `hlt` les interruptions sont ouvertes : un fil noyau
   preempte avant la section etait rearme, a sa reprise, a un instant perime
   et sur le bloc de son ancien coeur (invariant 2, os-primitives TCG 21/10 :
   `chevauchements=1 pire_chevauchement_us=5859`).
7. BOUCHAUD_COMPTA_INJECTION_V1 : l'injection du banc de C6a
   (`run_compta_injection.sh`) est INERTE par defaut -- fenetre nulle, ordre
   corrige -- et ne se regle que par `regle_injection_compta` ; l'ordre d'avant
   (`rearme_ancien_pour_banc`) n'est atteint que sous `INJECTION_ANCIEN`.

Fail-closed ; tests negatifs par mutation d'une copie de `src/`.
"""

import re
import shutil
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
SRC = RACINE / "src"

CHAMPS_TACHE = r"(?:last_account_ns|user_cpu_ns|kernel_cpu_ns|cpu_ns\[[^\]]*\])"
BLOC = r"(?:COMPTA_USER_NS|COMPTA_NOYAU_NS|COMPTA_DEBUT_NS|COMPTA_EN_NOYAU|CUMUL_USER_NS|CUMUL_NOYAU_NS)\[[^\]]*\]"
OPS = r"\.(?:range|store|fetch_add|fetch_sub|swap|echange|compare_exchange)\("
ECRITURE = re.compile(rf"(?:\.{CHAMPS_TACHE}|\b{BLOC}){OPS}")
FONCTIONS_CORPS = {"account_until", "frontiere_compta_bloc"}


def sans_commentaires(texte: str) -> str:
    """Remplace commentaires et chaines par des blancs, positions gardees."""
    sortie = []
    i, n = 0, len(texte)
    while i < n:
        if texte.startswith("//", i):
            j = texte.find("\n", i)
            j = n if j < 0 else j
            sortie.append(" " * (j - i))
            i = j
        elif texte[i] == '"':
            j = i + 1
            while j < n and texte[j] != '"':
                j += 2 if texte[j] == "\\" else 1
            sortie.append('"' + " " * (min(j, n - 1) - i - 1) + '"')
            i = j + 1
        else:
            sortie.append(texte[i])
            i += 1
    return "".join(sortie)


def fonctions(code: str):
    """(nom, debut, fin) des corps de `fn`, par appariement d'accolades."""
    for m in re.finditer(r"\bfn\s+(\w+)\s*(?:<[^>{]*>)?\s*\(", code):
        ouverture = code.find("{", m.end())
        point_virgule = code.find(";", m.end())
        if ouverture < 0 or (0 <= point_virgule < ouverture):
            continue
        profondeur, j = 0, ouverture
        while j < len(code):
            if code[j] == "{":
                profondeur += 1
            elif code[j] == "}":
                profondeur -= 1
                if profondeur == 0:
                    break
            j += 1
        yield m.group(1), ouverture, j


def sections(code: str):
    """Etendues des appels `compta_section(` (parentheses appariees)."""
    for m in re.finditer(r"\bcompta_section\s*\(", code):
        profondeur, j = 0, m.end() - 1
        while j < len(code):
            if code[j] == "(":
                profondeur += 1
            elif code[j] == ")":
                profondeur -= 1
                if profondeur == 0:
                    break
            j += 1
        yield m.start(), j


def enclosante(fns, position):
    meilleure = None
    for nom, debut, fin in fns:
        if debut <= position <= fin and (meilleure is None or debut > meilleure[1]):
            meilleure = (nom, debut, fin)
    return meilleure[0] if meilleure else None


def verifie(src: Path) -> list[str]:
    fautes = []
    vu_section = vu_lecture = vu_init = False
    vu_rearme = [False]
    for fichier in sorted(src.rglob("*.rs")):
        brut = fichier.read_text(encoding="utf-8")
        if not any(mot in brut for mot in ("_cpu_ns", "last_account_ns", "COMPTA_", "CUMUL_", "account_until", "frontiere_compta")):
            continue
        code = sans_commentaires(brut)
        relatif = fichier.relative_to(src.parent).as_posix()
        fns = list(fonctions(code))
        etendues = list(sections(code))

        def ligne(pos):
            return code.count("\n", 0, pos) + 1

        def en_section(pos):
            return any(a <= pos <= b for a, b in etendues)

        for m in ECRITURE.finditer(code):
            nom = enclosante(fns, m.start())
            if en_section(m.start()) or nom in FONCTIONS_CORPS:
                continue
            fautes.append(f"  {relatif}:{ligne(m.start())}  ecriture `{m.group(0)}` hors section "
                          f"(fonction `{nom}`) : un lecteur la verrait a moitie")

        for m in re.finditer(r"\baccount_until\s*\(", code):
            if code[max(0, m.start() - 3):m.start()] == "fn ":
                continue
            if not en_section(m.start()):
                fautes.append(f"  {relatif}:{ligne(m.start())}  `account_until` appele hors `compta_section`")
                continue
            # Regle 5 : l'instant du repli est lu dans CETTE section, avant l'appel.
            ouverture = max(a for a, b in etendues if a <= m.start() <= b)
            arg = re.match(r"\s*[^,()]+,\s*(\w+)\s*\)", code[m.end():])
            lu = arg and re.search(rf"\blet\s+{arg.group(1)}\s*=\s*crate::kernel::timer::monotonic_ns\(\)",
                                   code[ouverture:m.start()])
            if not lu:
                fautes.append(f"  {relatif}:{ligne(m.start())}  l'instant passe a `account_until` n'est pas lu "
                              "dans la section : un lecteur compterait la tranche au-dela du repli")
        for m in re.finditer(r"\bfrontiere_compta_bloc\s*\(", code):
            if code[max(0, m.start() - 3):m.start()] == "fn ":
                continue
            if enclosante(fns, m.start()) != "frontiere_compta":
                fautes.append(f"  {relatif}:{ligne(m.start())}  `frontiere_compta_bloc` appele hors `frontiere_compta`")

        for nom, debut, fin in fns:
            corps = code[debut:fin]
            if nom == "rearme_compta_apres_idle":
                vu_rearme[0] = True
                s = corps.find("compta_section(")
                lecture = corps.find("crate::kernel::timer::monotonic_ns()")
                coeur = corps.find("local_cpu()")
                deja = corps.find("task.last_account_ns != 0")
                if s < 0 or not (s < lecture and s < coeur):
                    fautes.append(f"  {relatif}  `rearme_compta_apres_idle` lit son instant ou son coeur hors de "
                                  "la section : une preemption entre les deux le rearme a un instant perime")
                if deja < 0 or not (s < deja < lecture):
                    fautes.append(f"  {relatif}  `rearme_compta_apres_idle` ecrase une tranche deja armee par "
                                  "l'election (tache preemptee apres le `hlt`)")
            if nom == "compta_section":
                vu_section = True
                if "without_interrupts" not in corps:
                    fautes.append(f"  {relatif}  `compta_section` ne masque plus les interruptions : "
                                  "le minuteur pourrait ouvrir une section imbriquee")
                if "debug_assert!(seq & 1 == 0" not in corps:
                    fautes.append(f"  {relatif}  `compta_section` ne verifie plus la parite")
                if "COMPTA_SEQ_CPU" not in corps:
                    fautes.append(f"  {relatif}  `compta_section` ne tient plus la sequence du coeur")
            if nom == "frontiere_compta" and "COMPTA_SEQ_CPU" not in corps:
                fautes.append(f"  {relatif}  `frontiere_compta` n'ecrit plus le bloc sous `COMPTA_SEQ_CPU`")
            if nom == "lecture_compta":
                vu_lecture = True
                if re.search(r"essais\s*>=", corps):
                    fautes.append(f"  {relatif}  `lecture_compta` borne ses relectures : elle rendrait une lecture dechiree")
        if "compta_seq: EcheanceAtomique::neuf(0)" in code:
            vu_init = True

    if not vu_section:
        fautes.append("  `compta_section` a disparu")
    if not vu_lecture:
        fautes.append("  `lecture_compta` a disparu")
    if not vu_init:
        fautes.append("  `Task::new` n'initialise plus `compta_seq`")
    if not vu_rearme[0]:
        fautes.append("  `rearme_compta_apres_idle` a disparu")
    # Regle 7 : injection inerte par defaut, posee par un seul reglage.
    c = (src / COMPTA).read_text(encoding="utf-8")
    if "static INJECTION_FENETRE_NS: AtomicU64 = AtomicU64::new(0);" not in c \
            or "static INJECTION_ANCIEN: AtomicBool = AtomicBool::new(false);" not in c \
            or len(re.findall(r"INJECTION_(FENETRE_NS|ANCIEN)\.store\(", c)) != 2 \
            or not re.search(r"pub fn regle_injection_compta\(fenetre_ns: u64, ancien: bool\) \{\s*"
                             r"INJECTION_ANCIEN\.store\(ancien, Ordering::Relaxed\);\s*"
                             r"INJECTION_FENETRE_NS\.store\(fenetre_ns, Ordering::Relaxed\);", c) \
            or len(re.findall(r"\brearme_ancien_pour_banc\(\)", c)) != 2 \
            or not re.search(r"if INJECTION_ANCIEN\.load\(Ordering::Relaxed\) \{\s*rearme_ancien_pour_banc\(\);\s*return;", c):
        fautes.append("  l'injection du banc C6a n'est plus inerte par defaut (ordre d'avant ou fenetre actifs sans reglage)")

    return fautes


COMPTA = "kernel/process/thread/comptabilite.rs"
INSTANT = ("        let now = crate::kernel::timer::monotonic_ns();\n"
           "        account_until(task, now);")


def mutation(fichier: str, remplacements) -> bool:
    """Vrai si la copie mutee est refusee ; faux si une mutation ne s'applique pas."""
    with tempfile.TemporaryDirectory() as tmp:
        copie = Path(tmp) / "src"
        shutil.copytree(SRC, copie)
        cible = copie / fichier
        texte = cible.read_text(encoding="utf-8")
        for avant, apres in remplacements:
            if avant not in texte:
                return False
            texte = texte.replace(avant, apres, 1)
        cible.write_text(texte, encoding="utf-8")
        return bool(verifie(copie))


def main() -> int:
    fautes = verifie(SRC)
    if fautes:
        print("comptabilite CPU : regle du verrou de sequence violee")
        print("\n".join(fautes))
        return 1
    negatifs = [
        # L'ancien ordre : l'instant lu avant la section.
        (COMPTA, [("fn account_slice_end(task: &Task) {\n",
                   "fn account_slice_end(task: &Task) {\n    let now = crate::kernel::timer::monotonic_ns();\n"),
                  (INSTANT, "        account_until(task, now);")]),
        # Un instant qui n'est pas une lecture d'horloge.
        (COMPTA, [(INSTANT, "        let now = task.last_account_ns.charge();\n        account_until(task, now);")]),
        # Le repli hors section.
        (COMPTA, [("    compta_section(task, || {\n        // BOUCHAUD_COMPTA_INSTANT_EN_SECTION_V1",
                   "    (|| {\n        // BOUCHAUD_COMPTA_INSTANT_EN_SECTION_V1")]),
        # Regle 6 : l'instant du rearmement lu avant la section (l'ancien ordre).
        (COMPTA, [("    let table = tasks();\n    let Some(task) = table.get(index) else {\n        return;\n    };\n"
                   "    compta_section(task, || {\n        // BOUCHAUD_COMPTA_REARME_EN_SECTION_V1",
                   "    let now = crate::kernel::timer::monotonic_ns();\n    let cpu = local_cpu();\n"
                   "    let table = tasks();\n    let Some(task) = table.get(index) else {\n        return;\n    };\n"
                   "    compta_section(task, || {\n        // BOUCHAUD_COMPTA_REARME_EN_SECTION_V1"),
                  ("        let now = crate::kernel::timer::monotonic_ns();\n        let cpu = local_cpu();\n"
                   "        note_armement_compta(cpu, now, ARMEMENT_APRES_IDLE",
                   "        note_armement_compta(cpu, now, ARMEMENT_APRES_IDLE")]),
        # Regle 6 : une tranche deja armee par l'election, ecrasee.
        (COMPTA, [("        if task.last_account_ns != 0 {\n            REARMES_DEJA_ARMES.fetch_add(1, Ordering::Relaxed);\n"
                   "            return;\n        }\n", "")]),
        # Regle 7 : l'ordre d'avant actif par defaut.
        (COMPTA, [("static INJECTION_ANCIEN: AtomicBool = AtomicBool::new(false);",
                   "static INJECTION_ANCIEN: AtomicBool = AtomicBool::new(true);")]),
        # Regle 7 : une fenetre injectee par defaut.
        (COMPTA, [("static INJECTION_FENETRE_NS: AtomicU64 = AtomicU64::new(0);",
                   "static INJECTION_FENETRE_NS: AtomicU64 = AtomicU64::new(2_000_000);")]),
        # Regle 7 : l'ordre d'avant appele sans condition.
        (COMPTA, [("    if INJECTION_ANCIEN.load(Ordering::Relaxed) {\n        rearme_ancien_pour_banc();\n        return;\n    }",
                   "    rearme_ancien_pour_banc();\n    if INJECTION_ANCIEN.load(Ordering::Relaxed) {\n        return;\n    }")]),
        # Une section qui ne masque plus les interruptions.
        (COMPTA, [("    interrupts::without_interrupts(|| {\n        let seq = task.compta_seq.charge();",
                   "    (|| {\n        let seq = task.compta_seq.charge();")]),
    ]
    for n, (f, r) in enumerate(negatifs, 1):
        if not mutation(f, r):
            print(f"comptabilite CPU : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"comptabilite CPU : toutes les ecritures sous sequence, instant du repli en section (negatifs={len(negatifs)})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
