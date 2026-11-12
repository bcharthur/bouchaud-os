#!/usr/bin/env python3
"""Paliers CI et provenance de l'artefact Ladybird (BOUCHAUD_CI_PALIERS_V2).

Un run de developpement peut reutiliser l'artefact Ladybird d'un run aux
entrees identiques et ne joue pas les sondes P13 : il ne doit JAMAIS pouvoir
passer pour une qualification, ni jouer un artefact non verifie.

Regles :

  1. ladybird-native-browser : le dispatch est une qualification par defaut ;
     les primitives P13 n'y sont appelees qu'en qualification ;
  2. le job de provenance joue la garde d'empreinte et recoit le palier ;
  3. le build garde un delai d'ETAPE sous le delai du job, sauvegarde ses
     caches (`!cancelled()`) AVANT de publier, et borne ses publications ;
  4. la reutilisation telecharge depuis le run source, puis REVERIFIE
     (liaison `entrees`) AVANT de republier ;
  5. le rendez-vous `artefact` exige un producteur et un seul ;
  6. chaque job qui joue l'artefact attend `artefact` ET exige explicitement
     `needs.artefact.result == 'success'` sous `!cancelled()` (sinon le
     success() implicite herite du saut de build/reutilise : run
     40554668634), puis le verifie avec la liaison publiee ;
  7. le contrat (tools/ci/contrat_paliers.py) et le workflow sont les memes :
     chaque epreuve du contrat est transmise au verdict par
     `R_X: ${{ needs.<job>.result }}` et attendue par lui ; une epreuve
     hors-run en developpement est exclue de ce palier par sa condition ; les
     epreuves requises dans les DEUX paliers sont epinglees ici ;
     verdict_convergence.py juge avec ce contrat et n'imprime CONVERGENCE_OK
     qu'en qualification ;
  8. p13-primitives-noyau.yml joue os-primitives sur TOUS les chemins noyau
     du declencheur Ladybird, hors branche d'audit (deja qualifiee), en push
     ET en pull_request (BOUCHAUD_P13_COUVERTURE_PR_V1) ;
  9. SIMULATION (tools/ci/simule_paliers.py) : pour chaque palier, toutes les
     epreuves requises sont jouees et le verdict est vert ; pour chaque panne
     injectee, le verdict est rouge ; le palier `artefact` ne joue que la
     provenance et le build ;
 10. PRODUCTION (BOUCHAUD_LADYBIRD_PRODUCTION_V1) : une qualification peut
     jouer un artefact reutilise ou attendu d'une production, JAMAIS sans le
     reverifier (le contrat exige `reutilise` dans les deux cas, et `attente`
     pour une production attendue ; `existe` n'est permis a aucun palier
     consommateur) ; la production est groupee par empreinte et jamais
     annulee ; elle ne joue ni epreuve, ni rendez-vous, ni verdict ; le build
     ne tourne pas pendant qu'une production est attendue ; la provenance peut
     declencher la production (actions: write) et l'audit `reconstruire`
     reste disponible.

Fail-closed ; tests negatifs par mutation.
"""
import re
import sys
import types
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(RACINE / "tools/ci"))
import simule_paliers  # noqa: E402

F = {
    "lb": RACINE / ".github/workflows/ladybird-native-browser.yml",
    "p13": RACINE / ".github/workflows/p13-primitives-noyau.yml",
    "verdict": RACINE / "tools/ci/verdict_convergence.py",
    "contrat": RACINE / "tools/ci/contrat_paliers.py",
}
# Epreuves requises en qualification ET en developpement : le palier de
# developpement garde les memes epreuves Ladybird. Les retirer d'un palier
# exige de modifier CETTE liste, visiblement.
REQUISES_PARTOUT = {
    "provenance", "artefact", "performance", "memoire-onglets", "ordre-worker", "crash-services",
    "compositor-piege", "endurance-longue", "browser-host-smoke", "cache", "wpt", "endurance",
    "endurance-kvm", "sites", "crash-rendu", "bureau-ring3",
}
HORS_RUN_DEV = {"primitives"}
Q = ("(github.event_name == 'workflow_dispatch' && inputs.palier == 'qualification') "
     "|| github.ref == 'refs/heads/audit/p13-console-sync-ci-20261024'")
LIAISON = 'tools/ci/verifie_artefact_ladybird.sh "${{ needs.artefact.outputs.liaison }}" native-browser'
ARTEFACT = "name: bouchaud-ladybird-native-browser"


def jobs(texte: str) -> dict:
    blocs, courant, dans = {}, None, False
    for ligne in texte.split("\n"):
        if ligne == "jobs:":
            dans = True
            continue
        if not dans:
            continue
        if ligne and not ligne.startswith(" ") and not ligne.startswith("#"):
            break
        if re.match(r"^  [A-Za-z0-9_-]+:$", ligne):
            courant = ligne.strip()[:-1]
            blocs[courant] = []
        elif courant:
            blocs[courant].append(ligne)
    return {k: "\n".join(v) for k, v in blocs.items()}


def etapes(bloc: str) -> list:
    """[(nom, texte)] ; nom = `name:` ou `uses:` de l'etape."""
    res = []
    for morceau in re.split(r"\n(?=      - )", bloc):
        if not morceau.startswith("      - "):
            continue
        m = re.search(r"^ {6}- name: (.*)$|^ {8}name: (.*)$", morceau, re.M)
        u = re.search(r"uses: (\S+)", morceau)
        res.append(((m.group(1) or m.group(2)).strip() if m else (u.group(1) if u else "?"), morceau))
    return res


def entete(bloc: str) -> str:
    return bloc.split("\n    steps:", 1)[0]


def index(liste, pred) -> int:
    return next((i for i, e in enumerate(liste) if pred(e)), -1)


def charge_contrat(texte: str):
    m = types.ModuleType("contrat_paliers_verifie")
    exec(compile(texte, "contrat_paliers.py", "exec"), m.__dict__)
    return m


def verifie(t: dict) -> list:
    f = []
    try:
        c = charge_contrat(t["contrat"])
    except Exception as exc:  # contrat illisible : tout le reste est sans objet
        return [f"contrat illisible : {exc}"]
    lb = t["lb"]
    j = jobs(lb)
    # 1
    if not re.search(r"workflow_dispatch:\n    inputs:\n      palier:\n(?:        .*\n)*?        default: qualification\n", lb):
        f.append("dispatch : la qualification n'est plus le palier par defaut")
    prim = entete(j.get("primitives", ""))
    if "uses: ./.github/workflows/os-primitives.yml" not in prim or f"&& ({Q}) && inputs.palier != 'artefact' }}}}" not in prim:
        f.append("primitives P13 appelees hors qualification, ou par une production")
    # P13_QUALIFICATION_STRICTE_V1 : en qualification, les sondes P13 sont
    # strictes -- la meme condition Q que le palier, pas la seule branche d'audit.
    if f"p13_strict: ${{{{ {Q} }}}}" not in j.get("primitives", ""):
        f.append("primitives : une qualification n'exige pas P13_STRICT_OK (sondes strictes reservees a l'audit)")
    # 2
    prov = j.get("provenance", "")
    if "python3 tools/ci/ladybird_empreinte.py garde" not in prov or "python3 tools/ci/ladybird_provenance.py" not in prov \
            or f"PALIER: ${{{{ ({Q}) && 'qualification' || 'developpement' }}}}" not in prov:
        f.append("provenance : garde d'empreinte ou palier absent")
    # 3
    build = j.get("build", "")
    if "needs.provenance.outputs.mode != 'reutilise' && needs.provenance.outputs.mode != 'attendu' && needs.provenance.outputs.mode != 'existe'" not in entete(build):
        f.append("build : ne suit pas la decision de provenance (repli build exige, aucun build pendant une production)")
    m_job = re.search(r"^    timeout-minutes: (\d+)$", entete(build), re.M)
    e = etapes(build)
    i_construit = index(e, lambda x: x[0] == "Construire Ladybird")
    m_etape = re.search(r"^        timeout-minutes: (\d+)$", e[i_construit][1], re.M) if i_construit >= 0 else None
    if not (m_job and m_etape and int(m_etape.group(1)) <= int(m_job.group(1)) - 20):
        f.append("build : pas de delai d'etape sous le delai du job (les caches mourraient avec lui)")
    i_pub = index(e, lambda x: x[0] == "Publier Ladybird natif")
    for nom in ("Sauvegarder le build incremental", "Sauvegarder ccache meme apres erreur"):
        i = index(e, lambda x, n=nom: x[0] == n)
        if i < 0 or i_pub < 0 or i > i_pub or "if: ${{ !cancelled()" not in e[i][1]:
            f.append(f"build : {nom} absent, conditionne au succes, ou apres la publication")
    for nom in ("Publier Ladybird natif", "Publier le corpus WPT"):
        i = index(e, lambda x, n=nom: x[0] == n)
        if i < 0 or not re.search(r"^        timeout-minutes: \d+$", e[i][1], re.M):
            f.append(f"build : {nom} sans delai")
    # 4
    reu = j.get("reutilise", "")
    er = etapes(reu)
    i_dl = index(er, lambda x: ARTEFACT in x[1]
                 and "run-id: ${{ needs.attente.outputs.source_run || needs.provenance.outputs.source_run }}" in x[1])
    i_ver = index(er, lambda x: "tools/ci/verifie_artefact_ladybird.sh entrees native-browser" in x[1])
    i_rep = index(er, lambda x: x[0] == "Republier Ladybird natif")
    if "(needs.provenance.outputs.mode == 'reutilise' || (needs.provenance.outputs.mode == 'attendu' && needs.attente.result == 'success'))" not in entete(reu) \
            or not re.search(r"^    needs: \[dedoublonne, provenance, attente\]$", entete(reu), re.M) or not (0 <= i_dl < i_ver < i_rep):
        f.append("reutilisation : republiee sans reverification des entrees")
    # 5
    art = j.get("artefact", "")
    for attendu in ('if [ "$R_BUILD" = success ] && [ "$R_REUTILISE" = skipped ]; then',
                    'elif [ "$R_REUTILISE" = success ] && [ "$R_BUILD" = skipped ] && { [ "$MODE" = reutilise ] || [ "$MODE" = attendu ]; }; then',
                    "echo liaison=tete", "echo liaison=entrees", "exit 1"):
        if attendu not in art:
            f.append("artefact : producteur unique non exige")
            break
    # 6
    for nom, bloc in j.items():
        if nom in ("build", "reutilise"):
            continue
        es = etapes(bloc)
        i_dl = index(es, lambda x: ARTEFACT in x[1] and "download-artifact" in x[1])
        if i_dl < 0:
            continue
        i_v = index(es, lambda x: LIAISON in x[1])
        e_job = entete(bloc)
        if not re.search(r"^    needs: \[dedoublonne, artefact\]$", e_job, re.M) or not i_dl < i_v \
                or "if: ${{ !cancelled() && needs.dedoublonne.outputs.reutilise != 'true' && needs.artefact.result == 'success' }}" not in e_job:
            f.append(f"{nom} : joue l'artefact sans l'attendre explicitement ou sans le verifier")
    # 7
    conv = j.get("convergence", "")
    if f"name: ${{{{ inputs.palier == 'artefact' && 'ladybird / verdict de production' || (({Q}) && 'ladybird / verdict de convergence' || 'ladybird / verdict integration developpement') }}}}" not in conv:
        f.append("verdict : meme nom en developpement qu'en qualification")
    besoins = re.search(r"^    needs: \[(.*)\]$", conv, re.M)
    attendus = {b.strip() for b in besoins.group(1).split(",")} if besoins else set()
    paires = [(e.cle, e.job) for e in c.EPREUVES] + list(c.PRODUCTEURS)
    for attendu in (f"BOUCHAUD_PALIER: ${{{{ ({Q}) && 'qualification' || 'developpement' }}}}",
                    "PROVENANCE_MODE: ${{ needs.provenance.outputs.mode }}"):
        if attendu not in conv:
            f.append(f"verdict : {attendu.split(':')[0]} non transmis")
    for cle, job in paires:
        if f"          {cle}: ${{{{ needs.{job}.result }}}}\n" not in conv or job not in attendus:
            f.append(f"verdict : {cle} (job {job}) non transmis ou non attendu -- contrat et workflow divergent")
    transmis = set(re.findall(r"^          (R_[A-Z0-9_]+): ", conv, re.M))
    if transmis != {cle for cle, _ in paires}:
        f.append(f"verdict : variables hors contrat {sorted(transmis ^ {cle for cle, _ in paires})}")
    requises = {e.job for e in c.EPREUVES if all(x == c.REQUIS for x in e.exigence.values())}
    if requises != REQUISES_PARTOUT:
        f.append(f"contrat : epreuves requises dans les deux paliers modifiees {sorted(requises ^ REQUISES_PARTOUT)}")
    hors = {e.job for e in c.EPREUVES if e.exigence.get(c.DEVELOPPEMENT) == c.HORS_RUN}
    if hors != HORS_RUN_DEV or any(e.exigence.get(c.QUALIFICATION) != c.REQUIS for e in c.EPREUVES):
        f.append("contrat : une epreuve n'est plus requise en qualification, ou une autre sort du developpement")
    for job in hors:
        if f"&& ({Q})" not in entete(j.get(job, "")):
            f.append(f"{job} : hors-run en developpement selon le contrat, mais pas exclu par sa condition")
    # 10 (contrat) : un artefact non construit dans le run est TOUJOURS reverifie.
    if any("existe" in modes for modes in c.MODES_PERMIS.values()) \
            or set(c.MODES_PERMIS.get(c.QUALIFICATION, ())) != {"construit", "reutilise", "attendu"}:
        f.append("contrat : qualification acceptee sans artefact produit ou reverifie dans le run")
    if c.PAR_MODE.get("reutilise") != ("reutilise",) or c.PAR_MODE.get("attendu") != ("attente", "reutilise") \
            or c.PAR_MODE.get("construit") != ("build",):
        f.append("contrat : un mode de provenance n'exige plus la reverification (reutilise) ou l'attente")
    # 10 (workflow) : production separee, groupee par empreinte, jamais annulee.
    if "group: ${{ inputs.palier == 'artefact' && format('ladybird-artefact-{0}', inputs.empreinte) || " not in lb \
            or "cancel-in-progress: ${{ inputs.palier != 'artefact' && " not in lb:
        f.append("production : non groupee par empreinte, ou annulable par un push")
    if not re.search(r"options: \[qualification, developpement, artefact\]\n", lb) \
            or not re.search(r"      reconstruire:\n(?:        .*\n)*?        type: boolean\n", lb):
        f.append("production : palier artefact ou audit `reconstruire` absent du dispatch")
    if "inputs.palier != 'artefact' && (needs.build.result == 'success' || needs.reutilise.result == 'success')" not in entete(art) \
            or not re.search(r"^    if: always\(\)$", entete(conv), re.M):
        f.append("production : joue des epreuves, ou le verdict peut etre saute")
    ev = etapes(conv)
    i_prod = index(ev, lambda x: x[0] == "Verdict de production")
    i_verdict = index(ev, lambda x: x[0] == "Verdict")
    if i_prod < 0 or "inputs.palier == 'artefact'" not in ev[i_prod][1] \
            or "ladybird_provenance.py verdict-production" not in ev[i_prod][1] \
            or i_verdict < 0 or "&& inputs.palier != 'artefact'" not in ev[i_verdict][1]:
        f.append("production : verdict de production absent, ou verdict de convergence rendu en production")
    att = j.get("attente", "")
    if "needs.provenance.outputs.mode == 'attendu'" not in entete(att) or "ladybird_provenance.py attends" not in att:
        f.append("attente : job absent ou detache de la decision")
    if not re.search(r"^      actions: write$", entete(prov), re.M) or "REF_PRODUCTION: " not in prov \
            or "RECONSTRUIRE: " not in prov or "EMPREINTE_DEMANDEE: " not in prov:
        f.append("provenance : ne peut pas declencher ni verifier la production")
    if c.NOM_VERDICT.get(c.QUALIFICATION) != "ladybird / verdict de convergence" \
            or c.NOM_VERDICT.get(c.DEVELOPPEMENT) == c.NOM_VERDICT.get(c.QUALIFICATION):
        f.append("contrat : noms de verdict confondus")
    v = t["verdict"]
    corps = v[v.find("def main()"):]
    if 'if os.environ.get("BOUCHAUD_PALIER") == contrat.QUALIFICATION:\n        print("BOUCHAUD_LADYBIRD_CONVERGENCE_OK")' not in corps \
            or v.count('print("BOUCHAUD_LADYBIRD_CONVERGENCE_OK")') != 1 or "juge = contrat.juge" not in v:
        f.append("verdict_convergence.py : ne juge pas avec le contrat, ou CONVERGENCE_OK hors qualification")
    # 8
    p13 = t["p13"]
    branches = re.search(r"\n  push:\n    branches: \[([^\]]*)\]", p13)
    if "uses: ./.github/workflows/os-primitives.yml" not in p13 or not branches or "audit/" in branches.group(1):
        f.append("p13-primitives-noyau : n'appelle pas os-primitives, ou double la branche d'audit")
    pousse = lb[lb.find("\n  push:"):lb.find("\n  workflow_dispatch:")]
    noyau = set(re.findall(r"^      - '(src/[^']+)'$", pousse, re.M))
    for evenement in ("pull_request", "push"):
        bloc = re.search(rf"\n  {evenement}:\n(?:    branches:[^\n]*\n)?    paths:\n((?:      - '[^']+'\n)+)", p13)
        couverts = set(re.findall(r"^      - '([^']+)'$", bloc.group(1), re.M)) if bloc else set()
        if not noyau or not noyau <= couverts:
            f.append(f"p13-primitives-noyau ({evenement}) : chemins noyau non couverts {sorted(noyau - couverts)}")
    # 9
    try:
        f += [f"simulation : {e}" for e in simule_paliers.controle(lb, c)]
    except Exception as exc:
        f.append(f"simulation impossible : {exc}")
    return f


def negatifs(t: dict) -> list:
    garde_ligne = "        run: python3 tools/ci/ladybird_empreinte.py garde\n"
    mutations = [
        ("dispatch en developpement par defaut", "lb", "        default: qualification\n", "        default: developpement\n"),
        ("primitives toujours", "lb", f"needs.dedoublonne.outputs.reutilise != 'true' && ({Q}) && inputs.palier != 'artefact' }}}}\n    uses:",
         "needs.dedoublonne.outputs.reutilise != 'true' && inputs.palier != 'artefact' }}\n    uses:"),
        ("primitives jouees par une production de la branche d'audit", "lb",
         f"&& ({Q}) && inputs.palier != 'artefact' }}}}\n    uses:", f"&& ({Q}) }}}}\n    uses:"),
        ("qualification non stricte", "lb", f"p13_strict: ${{{{ {Q} }}}}",
         "p13_strict: ${{ github.ref == 'refs/heads/audit/p13-console-sync-ci-20261024' }}"),
        ("garde d'empreinte retiree", "lb", garde_ligne, "        run: true\n"),
        ("build ignore la decision", "lb", "needs.provenance.outputs.mode != 'reutilise' && needs.provenance.outputs.mode != 'attendu' && needs.provenance.outputs.mode != 'existe' }}", "true }}"),
        ("delai d'etape retire", "lb", "        timeout-minutes: 150\n        run: |", "        run: |"),
        ("cache seulement sur succes", "lb", "if: ${{ !cancelled() && steps.ccache.outcome == 'success' }}",
         "if: ${{ success() && steps.ccache.outcome == 'success' }}"),
        ("publication non bornee", "lb", "        timeout-minutes: 20\n        uses: actions/upload-artifact@v4\n        with:\n          name: bouchaud-ladybird-native-browser\n          path: third_party/",
         "        uses: actions/upload-artifact@v4\n        with:\n          name: bouchaud-ladybird-native-browser\n          path: third_party/"),
        ("republication sans reverification", "lb", "        run: tools/ci/verifie_artefact_ladybird.sh entrees native-browser\n",
         "        run: true\n"),
        ("deux producteurs acceptes", "lb", '&& [ "$R_REUTILISE" = skipped ]; then', "; then"),
        ("consommateur non verifie", "lb", f"        run: {LIAISON}\n      - name: WPT sur Bouchaud OS",
         "        run: true\n      - name: WPT sur Bouchaud OS"),
        ("consommateur sans condition explicite (run 40554668634)", "lb",
         "    name: ladybird / sites reels\n    needs: [dedoublonne, artefact]\n",
         "    name: ladybird / sites reels\n    needs: artefact\n"),
        ("performance sans condition explicite", "lb",
         "    needs: [dedoublonne, browser-host-smoke]\n    if: ${{ !cancelled() && needs.dedoublonne.outputs.reutilise != 'true' && needs.browser-host-smoke.result == 'success' }}\n",
         "    needs: browser-host-smoke\n"),
        ("epreuve requise retiree des besoins du verdict", "lb",
         "crash-services, compositor-piege, endurance-longue]", "crash-services, compositor-piege]"),
        ("artefact non transmis au verdict", "lb", "          R_ARTEFACT: ${{ needs.artefact.result }}\n", ""),
        ("epreuve transmise sous un autre job", "lb", "          R_WPT: ${{ needs.wpt.result }}",
         "          R_WPT: ${{ needs.cache.result }}"),
        ("primitives jouees en developpement", "lb",
         "needs.dedoublonne.outputs.reutilise != 'true' && (" + Q + ") && inputs.palier != 'artefact' }}\n    uses:",
         "needs.dedoublonne.outputs.reutilise != 'true' && (true) && inputs.palier != 'artefact' }}\n    uses:"),
        ("contrat : WPT hors-run en developpement", "contrat",
         '_partout("R_WPT", "wpt", "WPT smoke (50 fichiers, compares a Linux)"),',
         'Epreuve("R_WPT", "wpt", "WPT", {QUALIFICATION: REQUIS, DEVELOPPEMENT: HORS_RUN}),'),
        ("contrat : primitives hors-run en qualification", "contrat",
         "{QUALIFICATION: REQUIS, DEVELOPPEMENT: HORS_RUN}),", "{QUALIFICATION: HORS_RUN, DEVELOPPEMENT: HORS_RUN}),"),
        ("contrat : qualification sur decision `existe`", "contrat",
         'MODES_PERMIS = {QUALIFICATION: ("construit", "reutilise", "attendu"),',
         'MODES_PERMIS = {QUALIFICATION: ("construit", "reutilise", "attendu", "existe"),'),
        ("contrat : production attendue sans reverification", "contrat",
         '    "attendu": ("attente", "reutilise"),', '    "attendu": ("attente",),'),
        ("production annulee par un push", "lb", "cancel-in-progress: ${{ inputs.palier != 'artefact' && ",
         "cancel-in-progress: ${{ "),
        ("production groupee par ref", "lb", "format('ladybird-artefact-{0}', inputs.empreinte)",
         "format('ladybird-artefact-{0}', github.ref)"),
        ("epreuves jouees en production", "lb",
         "inputs.palier != 'artefact' && (needs.build.result == 'success' || needs.reutilise.result == 'success')",
         "(needs.build.result == 'success' || needs.reutilise.result == 'success')"),
        ("verdict de convergence rendu en production", "lb",
         "        if: needs.dedoublonne.outputs.reutilise != 'true' && inputs.palier != 'artefact'\n",
         "        if: needs.dedoublonne.outputs.reutilise != 'true'\n"),
        ("verdict de production retire", "lb", "        run: python3 tools/ci/ladybird_provenance.py verdict-production\n",
         "        run: true\n"),
        ("verdict sautable en production", "lb", "    if: always()\n    runs-on: ubuntu-24.04\n    # Reflet",
         "    if: ${{ always() && inputs.palier != 'artefact' }}\n    runs-on: ubuntu-24.04\n    # Reflet"),
        ("reutilisation sans attente reussie", "lb", "(needs.provenance.outputs.mode == 'attendu' && needs.attente.result == 'success')",
         "(needs.provenance.outputs.mode == 'attendu')"),
        ("build pendant une production attendue", "lb",
         "needs.provenance.outputs.mode != 'reutilise' && needs.provenance.outputs.mode != 'attendu' && needs.provenance.outputs.mode != 'existe'",
         "needs.provenance.outputs.mode != 'reutilise' && needs.provenance.outputs.mode != 'existe'"),
        ("audit reconstruire retire", "lb", "      reconstruire:\n", "      reconstruire_x:\n"),
        ("attente non transmise au verdict", "lb", "          R_ATTENTE: ${{ needs.attente.result }}\n", ""),
        ("provenance sans droit de declencher", "lb", "      actions: write\n    outputs:\n      mode:",
         "      actions: read\n    outputs:\n      mode:"),
        ("contrat : skipped accepte pour un requis", "contrat",
         '        attendu = "success" if exigence == REQUIS else "skipped"\n        ok = resultat == attendu\n',
         '        attendu = "success" if exigence == REQUIS else "skipped"\n        ok = resultat in (attendu, "skipped")\n'),
        ("verdict au nom unique", "lb", "'ladybird / verdict integration developpement') }}\n    needs:",
         "'ladybird / verdict de convergence') }}\n    needs:"),
        ("palier non transmis", "lb", "          BOUCHAUD_PALIER: ", "          BOUCHAUD_PALIER_X: "),
        ("reutilisation non transmise", "lb", "          R_REUTILISE: ${{ needs.reutilise.result }}\n          R_ATTENTE",
         "          R_ATTENTE"),
        ("convergence en developpement", "verdict", 'if os.environ.get("BOUCHAUD_PALIER") == contrat.QUALIFICATION:\n        print("BOUCHAUD_LADYBIRD_CONVERGENCE_OK")',
         'if True:\n        print("BOUCHAUD_LADYBIRD_CONVERGENCE_OK")'),
        ("chemin noyau oublie (push)", "p13", "    branches: [main, 'claude/**', 'feat/**']\n    paths:\n"
         "      - '.github/workflows/os-primitives.yml'\n      - '.github/workflows/p13-primitives-noyau.yml'\n"
         "      - 'src/kernel/process/**'\n",
         "    branches: [main, 'claude/**', 'feat/**']\n    paths:\n"
         "      - '.github/workflows/os-primitives.yml'\n      - '.github/workflows/p13-primitives-noyau.yml'\n"),
        ("chemin noyau oublie (pull_request)", "p13", "  pull_request:\n    paths:\n"
         "      - '.github/workflows/os-primitives.yml'\n      - '.github/workflows/p13-primitives-noyau.yml'\n"
         "      - 'src/kernel/process/**'\n",
         "  pull_request:\n    paths:\n"
         "      - '.github/workflows/os-primitives.yml'\n      - '.github/workflows/p13-primitives-noyau.yml'\n"),
        ("audit double", "p13", "branches: [main, 'claude/**', 'feat/**']",
         "branches: [main, 'claude/**', 'feat/**', 'audit/p13-console-sync-ci-20261024']"),
    ]
    echecs = []
    for nom, cle, avant, apres in mutations:
        if t[cle].count(avant) != 1:
            echecs.append(f"mutation inapplicable ({t[cle].count(avant)} occurrences) : {nom}")
            continue
        m = dict(t)
        m[cle] = m[cle].replace(avant, apres, 1)
        if not verifie(m):
            echecs.append(f"mutation non detectee : {nom}")
    return echecs, len(mutations)


def main() -> int:
    t = {cle: p.read_text(encoding="utf-8") for cle, p in F.items()}
    fautes = verifie(t)
    echecs, n = negatifs(t)
    for faute in fautes + echecs:
        print(f"ECHEC {faute}")
    if fautes or echecs:
        return 1
    print(f"paliers CI et provenance Ladybird : ok ({n} mutations detectees)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
