#!/usr/bin/env python3
"""Une tache prete n'est jamais perdue par celui qui consomme son entree de
file (P13_ENTREE_VALIDE_JAMAIS_JETEE_V1).

C19 : banc console TCG SMP4 du 28/10 -- `[SCHED-ORPHELINE] tid=117 ... rq=3
en_file=Some(false) jetees_non_eligibles=1` : la sonde des ecrivains, prete,
hors de toute file, machine figee. `pick_next` avait consomme son entree, l'avait
jugee non eligible un instant, et ne l'avait remise nulle part.

Regles :

  1. `remet_si_perdue` ne remet que si la tache est PRETE, non revendiquee
     (`on_cpu < 0`), hors commutation, et absente de la file de son coeur ;
     elle compte chaque remise et reveille un coeur au repos ;
  2. l'election locale l'appelle sur CHAQUE entree rejetee, apres le compte
     des rejets ;
  3. le vol l'appelle quand la revendication echoue (l'identite n'est pas
     remise aveuglement chez le donneur) ;
  4. le compte est publie (`[SCHED-RESUME] ... remises=`) ;
  5. le banc (`banc-rejet-election`, hors `default`) n'existe que sous son
     option : sans elle, `banc_rejette` vaut `false` et la remise ne peut pas
     etre contournee ; son temoin (sans remise) DOIT signaler la tache perdue,
     et son bras corrige exige remises >= injectes > 0 et la sonde finie.

Fail-closed ; tests negatifs par mutation.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
F = {
    "ordo": RACINE / "src/kernel/process/thread/ordonnancement.rs",
    "diag": RACINE / "src/kernel/process/thread/diagnostic_stall.rs",
    "cargo": RACINE / "Cargo.toml",
    "banc": RACINE / "tools/ci/run_rejet_election.sh",
    "main": RACINE / "src/main.rs",
}
CONDITION = "    if tache.state != TaskState::Ready || tache.on_cpu >= 0 || tache.switching_out.charge() {\n        return;\n    }"
ABSENTE = "    if file.file_contient(mot) == Some(true) {\n        return;\n    }"
REMISE = "    if file.enqueue_bande(mot, bande_de(tache)) {\n        PICK_REMISES.fetch_add(1, Ordering::Relaxed);"
REVEIL = "        if rq != local_cpu() && cpu::is_idle(rq) {\n            smp::reschedule_cpu(rq);"
LOCAL = ("            PICK_JETEE_NON_ELIGIBLE[cpu.min(MAX_CPUS - 1)].fetch_add(1, Ordering::Relaxed);\n"
         "            remet_si_perdue(&tache, mot);")
VOL = ("        STEAL_REJECT_COURSE[cpu].fetch_add(1, Ordering::Relaxed);\n"
       "        remet_si_perdue(&candidate, mot);\n        return None;")
PUBLIE = "sur_coeur={} remises={}\","
OPTION = "#[cfg(feature = \"banc-rejet-election\")]"
CONTOURNE = "    " + OPTION + "\n    if BANC_SANS_REMISE.load(Ordering::Acquire) {\n        return;\n    }"
NEUTRE = ("#[cfg(not(feature = \"banc-rejet-election\"))]\n#[inline(always)]\n"
          "fn banc_rejette(_tache: &Task) -> bool {\n    false\n}")
TEMOIN = "exige \"temoin : une tache prete perdue est signalee ([SCHED-ORPHELINE])\" grep -aq '\\[SCHED-ORPHELINE\\] total=' \"$LOG\""
CORRIGE = "exige \"chaque rejet injecte remis (remises ${REMISES:-?} >= injectes ${INJECTES:-?})\" test \"${REMISES:-0}\" -ge \"${INJECTES:-1}\""
BANNIERE = (OPTION + "\n    crate::serial_println!(\"BOUCHAUD_IMAGE_TEMOIN feature=banc-rejet-election "
            "livrable=non qualifiable=non\");")
SONDE = "exige \"la sonde a fini, tous ses ecrivains a zero\""


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
                return source[debut:i + 1]
    return ""


def verifie(t: dict) -> list:
    fautes = []
    r = corps(t["ordo"], "fn remet_si_perdue(")
    ordre = [r.find(j) for j in (CONDITION, ABSENTE, REMISE, REVEIL)]
    if min(ordre) < 0 or ordre != sorted(ordre):
        fautes.append("remet_si_perdue : ne remet plus une tache prete, libre, hors commutation et absente, "
                      "ou ne compte plus, ou ne reveille plus un coeur au repos.")
    pick = corps(t["ordo"], "fn pick_next(")
    if LOCAL not in pick:
        fautes.append("pick_next : une entree rejetee par l'election locale n'est plus remise.")
    if VOL not in pick:
        fautes.append("pick_next : une entree volee dont la revendication echoue n'est plus remise.")
    if pick.count("remet_si_perdue(") != 2:
        fautes.append("pick_next : remet_si_perdue doit etre appelee exactement aux deux rejets.")
    if PUBLIE not in t["diag"]:
        fautes.append("diagnostic : le compte des remises n'est plus publie.")
    if not r.startswith("fn remet_si_perdue(tache: &Task, mot: u64) {\n" + CONTOURNE + "\n"):
        fautes.append("remet_si_perdue : le contournement du temoin n'est plus confine a l'option de banc.")
    for nom in ("static BANC_REJETS:", "static BANC_REJETS_FAITS:", "static BANC_SANS_REMISE:",
                "pub fn banc_regle_rejets(", "pub fn banc_rejets_faits(", "fn banc_rejette(tache: &Task)"):
        i = t["ordo"].find(nom)
        lignes = [l for l in t["ordo"][:max(i, 0)].splitlines() if not l.startswith("///")]
        if i < 0 or not lignes or lignes[-1] != OPTION:
            fautes.append(f"banc : {nom.strip()} n'est plus confine a l'option banc-rejet-election.")
    if NEUTRE not in t["ordo"] or "&& !banc_rejette(&tache)" not in pick:
        fautes.append("banc : sans l'option, banc_rejette ne vaut plus toujours false.")
    defaut = next((l for l in t["cargo"].splitlines() if l.startswith("default = ")), "")
    if "banc-rejet-election" in defaut or "\nbanc-rejet-election = []" not in t["cargo"]:
        fautes.append("Cargo.toml : l'option banc-rejet-election ne doit exister que hors de default.")
    if TEMOIN not in t["banc"] or CORRIGE not in t["banc"] or SONDE not in t["banc"]:
        fautes.append("run_rejet_election.sh : l'exigence du temoin ou du bras corrige a change.")
    if BANNIERE not in t["main"] or "grep -aq 'BOUCHAUD_IMAGE_TEMOIN feature=banc-rejet-election'" not in t["banc"]:
        fautes.append("banc : l'image de banc ne se declare plus non livrable, ou le banc ne l'exige plus.")
    return fautes


def negatifs(t: dict) -> tuple:
    mutations = [
        ("remise d'une tache non prete", "ordo", "tache.state != TaskState::Ready || ", ""),
        ("remise d'une tache revendiquee", "ordo", " || tache.on_cpu >= 0 || ", " || "),
        ("remise pendant la commutation", "ordo", " || tache.switching_out.charge() {\n        return;", " {\n        return;"),
        ("doublon non verifie", "ordo", ABSENTE + "\n", ""),
        ("remise non comptee", "ordo", "        PICK_REMISES.fetch_add(1, Ordering::Relaxed);\n", ""),
        ("coeur au repos non reveille", "ordo", "            smp::reschedule_cpu(rq);", "            let _ = rq;"),
        ("election locale perd l'entree", "ordo", LOCAL,
         "            PICK_JETEE_NON_ELIGIBLE[cpu.min(MAX_CPUS - 1)].fetch_add(1, Ordering::Relaxed);"),
        ("vol perd l'entree", "ordo", VOL, "        STEAL_REJECT_COURSE[cpu].fetch_add(1, Ordering::Relaxed);\n        return None;"),
        ("compte non publie", "diag", PUBLIE, "sur_coeur={}\","),
        ("contournement hors option", "ordo", CONTOURNE, "    if BANC_SANS_REMISE.load(Ordering::Acquire) {\n        return;\n    }"),
        ("etat du banc hors option", "ordo", OPTION + "\nstatic BANC_SANS_REMISE:", "static BANC_SANS_REMISE:"),
        ("rejet sans option", "ordo", "fn banc_rejette(_tache: &Task) -> bool {\n    false\n}",
         "fn banc_rejette(_tache: &Task) -> bool {\n    true\n}"),
        ("banc par defaut", "cargo", 'default = ["legacy-boot"]', 'default = ["legacy-boot", "banc-rejet-election"]'),
        ("temoin assoupli", "banc", TEMOIN, "exige \"temoin\" true"),
        ("remises non exigees", "banc", CORRIGE, "exige \"remises\" true"),
        ("sonde non exigee", "banc", SONDE, "exige \"sonde\""),
        ("banniere hors option", "main", BANNIERE, BANNIERE.split("\n", 1)[1]),
    ]
    echecs = []
    for nom, cle, avant, apres in mutations:
        if avant not in t[cle]:
            echecs.append(f"mutation inapplicable : {nom}")
            continue
        m = dict(t)
        m[cle] = t[cle].replace(avant, apres, 1)
        if not verifie(m):
            echecs.append(f"mutation non detectee : {nom}")
    return echecs, len(mutations)


def main() -> int:
    t = {k: p.read_text(encoding="utf-8") for k, p in F.items()}
    echecs, n = negatifs(t)
    fautes = verifie(t) + echecs
    if fautes:
        for f in fautes:
            print(f"verifie-entree-valide-jamais-jetee: {f}")
        return 1
    print(f"verifie-entree-valide-jamais-jetee: OK (election locale et vol remettent une tache prete et libre, "
          f"compte publie ; {n} negatifs)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
