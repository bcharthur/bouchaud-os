#!/usr/bin/env python3
"""Une barriere qui ne peut pas rougir ne protege rien.

# Le defaut que ce garde-fou encode

`check_budgets.py --journal` a passe au vert pendant toute une campagne en
n'ayant mesure STRICTEMENT RIEN. La trace `mm-ng6.log` ne portait aucun releve
periodique -- le scenario ne lancait jamais `smpstat`, et la boucle du
gestionnaire de fenetres ne tourne pas dans un scenario sans affichage. Les
treize grandeurs d'execution sortaient donc « absent du journal », et la
derniere ligne annoncait quand meme « ok budgets tenus ».

Une lecture rapide du journal de la barriere concluait « les budgets de latence
tiennent ». La verite etait « aucun budget de latence n'a ete lu ». C'est pire
que pas de barriere du tout : cela donne une garantie qui n'existe pas.

Ce garde-fou verifie les deux moities de la reparation, et il les verifie
SANS QEMU -- donc partout, et en une seconde :

  1. la barriere ECHOUE quand une grandeur requise manque ;
  2. le scenario qui l'alimente emet bien le releve.

La seconde compte autant que la premiere : rendre une grandeur obligatoire
sans que le scenario la produise deplacerait simplement le mensonge.
"""

import json
import re
import subprocess
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
BARRIERE = RACINE / "tools" / "ci" / "check_budgets.py"
REFERENCE = RACINE / "tools" / "ci" / "budgets" / "bkl-sites.json"
SCENARIO = RACINE / "tools" / "ci" / "run_mm_ng6.sh"

# Un releve periodique complet, tel que `log_smp_load` l'ecrit sur le port
# serie. Les lignes passees par `dmesg::log` portent le prefixe `[kernel] ` :
# le reproduire ICI garantit que les motifs de la barriere le tolerent.
RELEVE = """\
[kernel] [BKL-MAX-HOLD] ns=1200000 cpu=2 task=41 syscall=- site_acquisition=a origine=enter site_tenue=b
[kernel] [BKL-COMPTES] tenue_ns=900000 attente_ns=400000 attente_max_ns=2500000 \
attente_max=[origine=enter cpu=1 appel=x] reprise_ns=0 reprise_max_ns=0 \
spins=12 spins_irq_masquees=0 parks=3 wake_ipis=3 reveils_sans_acq=0 liberations_migrees=0 \
anomalies=0/0/0 proprietaire=-
[kernel] [BKL-DOMAINES] normaux=8123 regressions=0 debordements=0 premiere_regression=aucune
[SCHED-NG-PREEMPT] requests=900 safe=880 switches=870 blocked_bkl=4 blocked_preempt=2 blocked_ctx=1 max_defer_ns=3100000 attente_service_max_ns=2100011617
[SCHED-NG-LAT] count=5000 avg_ns=41000 max_ns=9100000 interactive_count=1800 interactive_max_ns=2700000 buckets_lt100us=10,lt500us=20,lt2ms=30,lt8ms=40,lt16ms=5,ge16ms=0
[SCHED-NG-CENTILES] classe=interactive count=1800 p50_ns=21000 p95_ns=310000 p99_ns=1400000 max_ns=2700000
[SCHED-NG-CENTILES] classe=normale count=3200 p50_ns=52000 p95_ns=900000 p99_ns=4100000 max_ns=9100000
[kernel] [SCHED-NG-FILE] cpu=0 interactives=3 normales=7 enfilees=9000 doublons=120 defilees=8900 volees=40 anti_famine=11
[kernel] [NET-TCP] retransmissions=2 rapides=1 echantillons_rtt=90 srtt_ms=12 rto_ms=200 busy_poll_tours=64000 sommeils=800
[kernel] [BKL-FUTEX] attentes=500 reveils=480 herites=120 profondeur_max=2
"""


def barriere(journal: Path) -> tuple[int, str]:
    fin = subprocess.run(
        [sys.executable, str(BARRIERE), "--journal", str(journal)],
        capture_output=True, text=True, cwd=RACINE,
    )
    return fin.returncode, fin.stdout + fin.stderr


def main() -> int:
    fautes: list[str] = []
    reference = json.loads(REFERENCE.read_text(encoding="utf-8"))
    execution = reference["execution"]
    requis = execution.get("_requis", [])
    budgets = [n for n in execution if not n.startswith("_")]

    if not requis:
        fautes.append(
            "aucune grandeur d'execution n'est declaree `_requis` : la barriere "
            "ne peut alors jamais rougir sur une trace muette, ce qui est le "
            "defaut que ce garde-fou existe pour empecher."
        )

    # --- 1. chaque grandeur requise doit exister, et etre EMISE quelque part --
    source = (RACINE / "tools" / "ci" / "check_budgets.py").read_text(encoding="utf-8")
    for nom in requis:
        if f'"{nom}"' not in source:
            fautes.append(f"{nom} est declaree requise mais inconnue de check_budgets.py")
        if nom not in execution:
            fautes.append(f"{nom} est declaree requise mais n'a aucun budget")

    # --- 2. une trace muette doit ECHOUER ------------------------------------
    muet = RACINE / "tools" / ".budget-muet.log"
    muet.write_text("rien qui ressemble a un releve\n", encoding="utf-8")
    try:
        code, sortie = barriere(muet)
        if code == 0:
            fautes.append(
                "une trace SANS aucun releve passe au vert. C'est exactement le "
                "defaut d'origine : la barriere annonce « budgets tenus » sans "
                "avoir lu une seule grandeur."
            )
        for nom in requis:
            if nom not in sortie:
                fautes.append(f"trace muette : {nom} n'est pas signalee comme requise")
    finally:
        muet.unlink(missing_ok=True)

    # --- 3. un releve complet doit PASSER, et se dire mesure ------------------
    plein = RACINE / "tools" / ".budget-plein.log"
    plein.write_text(RELEVE, encoding="utf-8")
    try:
        code, sortie = barriere(plein)
        if code != 0:
            fautes.append(
                "un releve periodique complet et DANS les budgets echoue :\n"
                + "\n".join("    " + l for l in sortie.splitlines())
            )
        attendu = f"{len(requis)}/{len(budgets)} grandeur(s) mesuree(s)"
        if attendu not in sortie:
            fautes.append(
                f"la ligne de resume ne dit pas ce qui a ete mesure "
                f"(attendu « {attendu} ») : « budgets tenus » seul se lit comme "
                f"« tout a ete verifie ». Sortie : {sortie.strip()!r}"
            )
    finally:
        plein.unlink(missing_ok=True)

    # --- 4. un depassement d'une grandeur requise doit ECHOUER ----------------
    # Sans cela, « requise » ne voudrait dire que « presente ».
    creve = RACINE / "tools" / ".budget-creve.log"
    plafond = float(execution["ready_latency_interactive_p99_ms"])
    depasse = int((plafond + 10) * 1_000_000)
    creve.write_text(
        RELEVE.replace("p99_ns=1400000", f"p99_ns={depasse}"), encoding="utf-8"
    )
    try:
        code, sortie = barriere(creve)
        if code == 0:
            fautes.append(
                "un p99 interactif au-dela de son plafond passe au vert : la "
                "grandeur est lue mais son budget n'est pas applique."
            )
    finally:
        creve.unlink(missing_ok=True)

    # --- 5. le scenario qui alimente la barriere doit emettre le releve -------
    #
    # Rendre une grandeur obligatoire sans que le scenario la produise ne
    # corrigerait rien : la barriere rougirait en permanence, donc on
    # l'eteindrait. Les deux moities tiennent ensemble.
    texte = SCENARIO.read_text(encoding="utf-8")
    autorun = re.search(r"<<'AUTORUN'\n(.*?)\nAUTORUN", texte, re.S)
    if autorun is None:
        fautes.append(f"{SCENARIO.name} : bloc autorun introuvable")
    elif not re.search(r"^\s*smpstat\s*$", autorun.group(1), re.M):
        fautes.append(
            f"{SCENARIO.name} : l'autorun n'appelle pas `smpstat`, donc la trace "
            f"ne portera aucun releve periodique et les grandeurs requises "
            f"seront absentes. C'est l'etat qui a rendu la barriere muette."
        )

    if fautes:
        print("budgets d'execution : la barriere ne protege pas ce qu'elle annonce")
        for f in fautes:
            print(f"  - {f}")
        return 1

    print(f"ok  {len(requis)} grandeur(s) requise(s) ; trace muette rejetee ; "
          f"depassement rejete ; scenario mm-ng6 emet le releve")
    return 0


if __name__ == "__main__":
    sys.exit(main())
