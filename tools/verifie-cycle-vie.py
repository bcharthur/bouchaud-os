#!/usr/bin/env python3
"""Le cycle de vie d'une tache : un zombie ne ressuscite jamais.

BOUCHAUD_CYCLE_DE_VIE_V1

# Le defaut

`scheduler-ng-banc` (ligne de base Scheduler NG, SMP2/4/8/16) : apres un
`exit_group`, 210 a 269 compteurs de fils MORTS bougeaient encore apres la
recolte de leur processus. L'etat d'une tache s'ecrivait par simple store :
le frere tue pendant qu'il entrait dans `nanosleep` ecrasait `Zombie` par
`Blocked`, l'echeance le remettait `Ready`, il retournait en espace
utilisateur.

# La regle (`kernel::cycle_vie`, explore par `tools/smp/test_cycle_vie.rs`)

  1. l'etat ne s'ecrit que par les transitions nommees de `EtatAtomique`
     (`endort`, `reveille`, `tue_parquee`, `meurt`), toutes des CAS ou un
     echange vers Zombie : aucune ne quitte Zombie, et il n'existe plus
     d'ecriture libre (`range`, `echange`) ;
  2. une mort imposee a une AUTRE tache passe par `condamne`, qui publie la
     condamnation AVANT de lire l'etat et applique `action_tueur` ;
  3. une tache publie `Blocked` (`endort`) AVANT de relire sa condamnation ;
  4. les frontieres tuent la tache condamnee : sortie d'appel systeme et de
     faute (`retire_current_if_zombie`), preemption depuis l'espace
     utilisateur (`preempt_from_irq`), premier passage en espace utilisateur
     (`task_trampoline`), mise en route sur un coeur (drapeau de retraite) ;
  5. les attentes `WaitQueue` sont interruptibles par defaut ; les neuf
     attentes qui tiennent une ressource du noyau prennent `wait_noyau` ;
  6. `execve` et `exit_group` attendent la mort des freres condamnes
     (attente non interruptible) avant de remplacer l'image ou de rapporter
     la fin du processus ;
  7. le banc hote rejoue l'ancien et le nouveau protocole.

Fail-closed ; neuf tests negatifs.
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
MODELES = "src/kernel/process/thread/modeles.rs"
COMPTA = "src/kernel/process/thread/comptabilite.rs"
BLOCAGE = "src/kernel/process/thread/blocage.rs"
SOMMEIL = "src/kernel/process/thread/sommeil.rs"
VIE = "src/kernel/process/thread/lifecycle.rs"
COMMUT = "src/kernel/process/thread/commutation.rs"
PREEMPT = "src/kernel/process/thread/preemption.rs"
ATTENTE = "src/kernel/sync/wait_queue/attente.rs"
PROC = "src/compat/linux/proc.rs"
TEST = "tools/smp/test_cycle_vie.rs"
REGLE = "src/kernel/scheduler/cycle_vie.rs"
# Les attentes qui tiennent une ressource du noyau : (fichier, nombre exact).
NOYAU = {
    "src/kernel/sync/sleep_mutex.rs": 1,
    "src/kernel/memory/page_cache.rs": 1,
    "src/kernel/memory/shared.rs": 5,
    "src/kernel/process/thread/faute_memoire.rs": 1,
    "src/drivers/block/ata.rs": 1,
}
FICHIERS = (MODELES, COMPTA, BLOCAGE, SOMMEIL, VIE, COMMUT, PREEMPT, ATTENTE, PROC, TEST, REGLE) + tuple(NOYAU)


def sans_commentaires(texte: str) -> str:
    return "\n".join(l.split("//", 1)[0] for l in texte.splitlines())


def corps(texte: str, nom: str) -> str:
    m = re.search(r"\bfn\s+" + re.escape(nom) + r"\s*(?:<[^>{]*>)?\(", texte)
    if not m:
        return ""
    ouverture = texte.find("{", m.end())
    profondeur = 0
    for i in range(ouverture, len(texte)):
        if texte[i] == "{":
            profondeur += 1
        elif texte[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return texte[ouverture:i + 1]
    return ""


def avant(texte: str, a: str, b: str) -> bool:
    i, j = texte.find(a), texte.find(b)
    return 0 <= i < j


def verifie(racine: Path) -> list[str]:
    try:
        src = {f: sans_commentaires((racine / f).read_text(encoding="utf-8")) for f in FICHIERS}
        tout_src = {p: sans_commentaires(p.read_text(encoding="utf-8"))
                    for p in (racine / "src").rglob("*.rs")}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    fautes = []

    # 1. Aucune ecriture libre de l'etat, nulle part.
    for chemin, texte in tout_src.items():
        if re.search(r"\.state\s*\.\s*(range|echange)\s*\(", texte):
            fautes.append(f"{chemin.relative_to(racine)} : ecriture libre de l'etat d'une tache")
    etat = src[MODELES][src[MODELES].find("impl EtatAtomique"):]
    etat = etat[:etat.find("\nimpl ", 10)]
    if re.search(r"pub fn (range|echange)\b", etat):
        fautes.append(f"{MODELES} : EtatAtomique expose de nouveau une ecriture libre")
    for nom, depuis, vers in (("endort", "Ready", "Blocked"), ("reveille", "Blocked", "Ready"),
                              ("tue_parquee", "Blocked", "Zombie")):
        if f"self.transite(TaskState::{depuis}, TaskState::{vers})" not in corps(etat, nom):
            fautes.append(f"{MODELES} : {nom} n'est plus le CAS {depuis} -> {vers}")
    if "TaskState::Zombie" not in corps(etat, "meurt"):
        fautes.append(f"{MODELES} : meurt ne mene plus a Zombie")
    if "SeqCst" not in corps(etat, "transite"):
        fautes.append(f"{MODELES} : les transitions ne sont plus totalement ordonnees")

    # 2. Mort imposee : condamnation, puis decision pure.
    mz = corps(src[COMPTA], "marque_zombie")
    if "meurt_soi_meme(task)" not in mz or "condamne(task)" not in mz:
        fautes.append(f"{COMPTA} : marque_zombie ne distingue plus la courante des autres")
    cd = corps(src[COMPTA], "condamne")
    if not avant(cd, "task.condamnee.condamne()", "task.state.charge()") or "action_tueur(" not in cd:
        fautes.append(f"{COMPTA} : condamne ne publie plus la condamnation avant de lire l'etat")
    if ".meurt()" in cd:
        fautes.append(f"{COMPTA} : condamne ecrit Zombie sur une tache qui s'execute")

    # 3. La tache publie Blocked AVANT de relire sa condamnation.
    pp = corps(src[BLOCAGE], "publie_parking")
    if not avant(pp, "task.attente_interruptible.range(interruptible)", "task.state.endort()") \
            or not avant(pp, "task.state.endort()", "est_condamnee()"):
        fautes.append(f"{BLOCAGE} : le parking ne relit plus la condamnation apres sa publication")
    # BOUCHAUD_SOMMEIL_SIGNAL_V1 : le corps du sommeil vit dans `dort_jusqua`
    # (une echeance fixe, pour que `nanosleep` se rendorme jusqu'a la MEME
    # echeance apres un reveil sans effet) ; `dort` et
    # `sleep_ticks_signalable` y passent tous deux, interruptibles.
    if "dort(ticks, true)" not in corps(src[SOMMEIL], "sleep_ticks") \
            or "dort_jusqua(" not in corps(src[SOMMEIL], "dort") \
            or "dort_jusqua(deadline, ticks, true)" not in corps(src[SOMMEIL], "sleep_ticks_signalable") \
            or "publie_parking(interruptible)" not in corps(src[SOMMEIL], "dort_jusqua"):
        fautes.append(f"{SOMMEIL} : le sommeil ne passe plus par le parking interruptible")
    if "publie_attente_interruptible()" not in corps(src[PROC], "sys_wait4"):
        fautes.append(f"{PROC} : wait4 ne passe plus par l'attente interruptible")

    # 4. Les frontieres.
    if "condamnee" not in corps(src[VIE], "retire_current_if_zombie") \
            or "meurt_a_la_frontiere()" not in corps(src[VIE], "retire_current_if_zombie"):
        fautes.append(f"{VIE} : la sortie d'appel systeme ne tue plus une tache condamnee")
    if "task.condamnee.est_condamnee()" not in corps(src[COMMUT], "finalise_task_running"):
        fautes.append(f"{COMMUT} : la mise en route ne pose plus la retraite d'une condamnee")
    tt = corps(src[COMMUT], "task_trampoline")
    if not avant(tt, "meurt_a_la_frontiere()", "resume_usermode"):
        fautes.append(f"{COMMUT} : un fil condamne entre en espace utilisateur")
    pf = corps(src[PREEMPT], "preempt_from_irq")
    if "condamnee_courante()" not in pf or "!current_is_kernel_task()" not in pf:
        fautes.append(f"{PREEMPT} : la preemption utilisateur ne tue plus une tache condamnee")

    # 5. Attentes interruptibles par defaut, noyau explicites.
    if "self.attend(ticket, None, true)" not in corps(src[ATTENTE], "wait") \
            or "self.attend(ticket, None, false)" not in corps(src[ATTENTE], "wait_noyau"):
        fautes.append(f"{ATTENTE} : l'interruptibilite par defaut des WaitQueue a change")
    att = corps(src[ATTENTE], "attend")
    if not avant(att, "drop(inscrit);", "meurt_au_parking()"):
        fautes.append(f"{ATTENTE} : une tache qui meurt au parking garde son inscription")
    for fichier, n in NOYAU.items():
        vus = len(re.findall(r"\.wait_noyau\(", src[fichier]))
        if vus != n:
            fautes.append(f"{fichier} : {vus} attente(s) noyau au lieu de {n}")

    # 6. execve et exit_group attendent la mort des freres -- sans pouvoir
    #    mourir eux-memes dans cette attente.
    if not avant(src[PROC], "task::terminate_sibling_threads();", "task::attend_extinction_freres();"):
        fautes.append(f"{PROC} : execve remplace l'image avant la mort des freres")
    eg = corps(src[VIE], "exit_group")
    if not avant(eg, "marque_zombie(task);", "attend_extinction_freres();") \
            or not avant(eg, "attend_extinction_freres();", "exit_current(code)"):
        fautes.append(f"{VIE} : exit_group rapporte la fin du processus avant la mort des freres")
    if "sleep_ticks_noyau(1)" not in corps(src[VIE], "attend_extinction_freres"):
        fautes.append(f"{VIE} : l'attente des freres est interruptible")

    # 7. Banc hote.
    if 'src/kernel/scheduler/cycle_vie.rs' not in src[TEST] or "Protocole::Ancien" not in src[TEST]:
        fautes.append(f"{TEST} : le banc hote ne rejoue plus l'ancien protocole")
    if "(Etat::Bloque, Transition::Reveille) => Some(Etat::Pret)" not in src[REGLE]:
        fautes.append(f"{REGLE} : la table des transitions a change")
    return fautes


def mutation(fichier: str, avant_: str, apres: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        copie = Path(tmp)
        for p in (RACINE / "src").rglob("*.rs"):
            dest = copie / p.relative_to(RACINE)
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text(p.read_text(encoding="utf-8"), encoding="utf-8")
        dest = copie / TEST
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text((RACINE / TEST).read_text(encoding="utf-8"), encoding="utf-8")
        cible = copie / fichier
        texte = cible.read_text(encoding="utf-8")
        if avant_ not in texte:
            return False
        cible.write_text(texte.replace(avant_, apres, 1), encoding="utf-8")
        return bool(verifie(copie))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("cycle de vie : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        # L'ancienne ecriture libre revient dans le sommeil.
        (SOMMEIL, "    match publie_parking(interruptible) {", "    current().state.range(TaskState::Blocked);\n    match publie_parking(interruptible) {"),
        # Le tueur ecrit Zombie sur une tache qui s'execute.
        (COMPTA, "            ActionTueur::Rien | ActionTueur::AttendSonEvenement => return,",
         "            ActionTueur::Rien | ActionTueur::AttendSonEvenement => { task.state.meurt(); return }"),
        # La tache relit sa condamnation AVANT de publier Blocked.
        (BLOCAGE, "    if !task.state.endort() {\n        ENDORMISSEMENTS_REFUSES",
         "    let _ = task.condamnee.est_condamnee();\n    if !task.state.endort() {\n        ENDORMISSEMENTS_REFUSES"),
        # La sortie d'appel systeme oublie la condamnation.
        (VIE, "    if current().condamnee.est_condamnee() {\n        meurt_a_la_frontiere();\n    }", ""),
        # Un fil condamne entre en espace utilisateur.
        (COMMUT, "        if current().condamnee.est_condamnee() {\n            meurt_a_la_frontiere();\n        }", ""),
        # Le verrou dormant devient interruptible.
        ("src/kernel/sync/sleep_mutex.rs", ".wait_noyau(ticket)", ".wait(ticket)"),
        # execve n'attend plus ses freres.
        (PROC, "    task::attend_extinction_freres();\n", ""),
        # La preemption utilisateur ne tue plus.
        (PREEMPT, "&& condamnee_courante()", "&& false"),
        # exit_group n'attend plus ses freres.
        (VIE, "    attend_extinction_freres();\n\n    // `exit_group` termine", "\n    // `exit_group` termine"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"cycle de vie : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"CYCLE_DE_VIE_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
