#!/usr/bin/env python3
"""Simulation de l'ordonnancement GitHub du workflow Ladybird (BOUCHAUD_CI_PALIERS_V2).

Pourquoi : le run 40554668634 a montre qu'un workflow peut etre syntaxiquement
valide (actionlint), conforme a la politique textuelle, et pourtant sauter
toutes ses epreuves. Les jobs consommateurs dependaient de `artefact` sans
condition ; GitHub les enveloppe alors d'un success() implicite qui porte sur
TOUS les ancetres, or `build` et `reutilise` sont exclusifs : l'un est
toujours saute, et le saut se propage. Quinze jobs sautes, verdict rouge (a
raison), aucune epreuve jouee.

Ce module rejoue le graphe `needs`/`if` du workflow pour un scenario (palier,
decision de provenance, pannes injectees) avec cette semantique :

  * job sans `if`, ou `if` sans fonction d'etat : il tourne si TOUS ses
    ancetres, directs et transitifs, ont conclu `success` (et si l'expression
    est vraie) ;
  * `always()` vrai ; `cancelled()` faux (aucune annulation simulee) ;
    `success()` / `failure()` portent sur tous les ancetres ;
  * `needs.X.*` n'est lisible que pour un besoin DIRECT (sinon : erreur) ;
  * un job qui tourne conclut `success`, sauf panne injectee.

Puis il evalue l'environnement du job de verdict et le soumet a
`contrat_paliers.juge` : la meme fonction que le vrai verdict.
"""
from __future__ import annotations

import re
from pathlib import Path

import contrat_paliers as contrat

WORKFLOW = Path(__file__).resolve().parents[2] / ".github/workflows/ladybird-native-browser.yml"


class SimulationErreur(RuntimeError):
    pass


# --- lecture du workflow (texte, sans dependance YAML) ----------------------

def jobs(texte: str) -> dict[str, list[str]]:
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
    return blocs


def entete(bloc: list[str]) -> dict:
    j = {"needs": [], "if": None, "name": None}
    for ligne in bloc:
        if ligne.startswith("    steps:"):
            break
        m = re.match(r"^    needs: (.*)$", ligne)
        if m:
            v = m.group(1).strip()
            j["needs"] = [x.strip() for x in v.strip("[]").split(",")] if v.startswith("[") else [v]
        m = re.match(r"^    if: (.*)$", ligne)
        if m:
            j["if"] = m.group(1).strip()
        m = re.match(r"^    name: (.*)$", ligne)
        if m:
            j["name"] = m.group(1).strip()
    return j


def env_verdict(bloc: list[str]) -> dict[str, str]:
    """Variables de l'etape `Verdict` du job de convergence."""
    env, dans_etape, dans_env = {}, False, False
    for ligne in bloc:
        if re.match(r"^      - name: Verdict$", ligne):
            dans_etape, dans_env = True, False
            continue
        if dans_etape and ligne.startswith("      - "):
            break
        if dans_etape and ligne.startswith("        env:"):
            dans_env = True
            continue
        if dans_env:
            m = re.match(r"^          ([A-Z_]+): (.*)$", ligne)
            if m:
                env[m.group(1)] = m.group(2).strip()
            elif not ligne.startswith("          "):
                dans_env = False
    if not env:
        raise SimulationErreur("etape `Verdict` sans environnement")
    return env


# --- expressions GitHub -----------------------------------------------------

JETON = re.compile(r"\s*(?:(\|\||&&|==|!=|!|\(|\)|,)|'((?:[^']|'')*)'|([A-Za-z_][A-Za-z0-9_.\-]*))")


def jetons(expr: str) -> list[tuple[str, str]]:
    res, pos = [], 0
    expr = expr.strip()
    while pos < len(expr):
        m = JETON.match(expr, pos)
        if not m or m.end() == pos:
            raise SimulationErreur(f"expression illisible a {pos} : {expr!r}")
        if m.group(1):
            res.append(("op", m.group(1)))
        elif m.group(2) is not None:
            res.append(("str", m.group(2).replace("''", "'")))
        else:
            res.append(("id", m.group(3)))
        pos = m.end()
    return res


class Evaluateur:
    def __init__(self, contexte, fonctions):
        self.ctx, self.fn = contexte, fonctions

    def evalue(self, expr: str):
        expr = expr.strip()
        if expr.startswith("${{") and expr.endswith("}}"):
            expr = expr[3:-2]
        self.t, self.i = jetons(expr), 0
        v = self.ou()
        if self.i != len(self.t):
            raise SimulationErreur(f"reste non lu dans {expr!r}")
        return v

    def voit(self, op):
        return self.i < len(self.t) and self.t[self.i] == ("op", op)

    def ou(self):
        v = self.et()
        while self.voit("||"):
            self.i += 1
            d = self.et()
            v = v if vrai(v) else d
        return v

    def et(self):
        v = self.comp()
        while self.voit("&&"):
            self.i += 1
            d = self.comp()
            v = d if vrai(v) else v
        return v

    def comp(self):
        v = self.unaire()
        while self.voit("==") or self.voit("!="):
            op = self.t[self.i][1]
            self.i += 1
            d = self.unaire()
            egal = norm(v) == norm(d)
            v = egal if op == "==" else not egal
        return v

    def unaire(self):
        if self.voit("!"):
            self.i += 1
            return not vrai(self.unaire())
        return self.primaire()

    def primaire(self):
        genre, val = self.t[self.i]
        self.i += 1
        if (genre, val) == ("op", "("):
            v = self.ou()
            if not self.voit(")"):
                raise SimulationErreur("parenthese non fermee")
            self.i += 1
            return v
        if genre == "str":
            return val
        if genre == "id":
            if self.voit("("):
                self.i += 1
                if not self.voit(")"):
                    raise SimulationErreur(f"{val}() avec arguments non simule")
                self.i += 1
                if val not in self.fn:
                    raise SimulationErreur(f"fonction inconnue {val}()")
                return self.fn[val]()
            if val in ("true", "false"):
                return val == "true"
            return self.ctx(val)
        raise SimulationErreur(f"jeton inattendu {val!r}")


def vrai(v) -> bool:
    return bool(v) and v != ""


def norm(v):
    return v.lower() if isinstance(v, str) else v


STATUT = re.compile(r"\b(success|failure|always|cancelled)\(\)")


# --- scenarios ---------------------------------------------------------------

class Scenario:
    def __init__(self, nom, evenement, palier_entree="", ref="refs/heads/dev", mode="reutilise",
                 pannes=(), head_ref=""):
        self.nom, self.evenement, self.palier_entree, self.ref = nom, evenement, palier_entree, ref
        # pannes : noms (conclusion `failure`) ou {nom: conclusion} -- `skipped`
        # simule un job requis que sa condition a fait sauter.
        self.pannes = dict(pannes) if isinstance(pannes, dict) else dict.fromkeys(pannes, "failure")
        self.mode, self.head_ref = mode, head_ref


def simule(texte: str, sc: Scenario, c=contrat) -> dict:
    blocs = jobs(texte)
    graphe = {nom: entete(b) for nom, b in blocs.items()}
    for nom, j in graphe.items():
        for b in j["needs"]:
            if b not in graphe:
                raise SimulationErreur(f"{nom} attend un job inconnu {b}")
    ancetres = {}

    def anc(nom, pile=()):
        if nom in pile:
            raise SimulationErreur(f"cycle via {nom}")
        if nom not in ancetres:
            a = set()
            for b in graphe[nom]["needs"]:
                a |= {b} | anc(b, pile + (nom,))
            ancetres[nom] = a
        return ancetres[nom]

    resultats, sorties = {}, {}

    def sorties_de(nom):
        if resultats.get(nom) != "success":
            return {}
        if nom == "dedoublonne":
            return {"reutilise": "false", "source_run": ""}
        if nom == "provenance":
            return {"mode": sc.mode, "source_run": "1" if sc.mode == "reutilise" else "", "empreinte": "e"}
        if nom == "artefact":
            if resultats.get("build") == "success" and resultats.get("reutilise") == "skipped":
                return {"liaison": "tete"}
            if resultats.get("reutilise") == "success" and resultats.get("build") == "skipped" \
                    and sorties.get("provenance", {}).get("mode") == "reutilise":
                return {"liaison": "entrees"}
            return {}
        return {}

    def contexte_pour(nom):
        directs = set(graphe[nom]["needs"]) if nom else set(graphe)

        def ctx(ident):
            morceaux = ident.split(".")
            if ident == "github.event_name":
                return sc.evenement
            if ident == "github.ref":
                return sc.ref
            if ident == "github.head_ref":
                return sc.head_ref
            if ident == "inputs.palier":
                return sc.palier_entree if sc.evenement == "workflow_dispatch" else ""
            if morceaux[0] == "needs":
                job = morceaux[1]
                if job not in directs:
                    raise SimulationErreur(f"{nom} lit needs.{job} sans l'attendre directement")
                if morceaux[2:] == ["result"]:
                    return resultats[job]
                if len(morceaux) == 4 and morceaux[2] == "outputs":
                    return sorties.get(job, {}).get(morceaux[3], "")
            if morceaux[0] in ("github", "inputs", "vars", "secrets", "env"):
                return ""
            raise SimulationErreur(f"contexte non simule : {ident}")
        return ctx

    for nom in ordre_topologique(graphe):
        j = graphe[nom]
        a = anc(nom)
        fonctions = {
            "success": lambda a=a: all(resultats[x] == "success" for x in a),
            "failure": lambda a=a: any(resultats[x] == "failure" for x in a),
            "always": lambda: True,
            "cancelled": lambda: False,
        }
        expr = j["if"] or "success()"
        if expr.startswith("${{"):
            expr = expr[3:-2]
        if not STATUT.search(expr):
            expr = f"success() && ({expr})"
        tourne = vrai(Evaluateur(contexte_pour(nom), fonctions).evalue(expr))
        resultats[nom] = (sc.pannes.get(nom, "success")) if tourne else "skipped"
        sorties[nom] = sorties_de(nom)

    conv = graphe.get("convergence")
    if not conv:
        raise SimulationErreur("job de convergence absent")
    ctx = contexte_pour("convergence")
    ev = Evaluateur(ctx, {"always": lambda: True, "cancelled": lambda: False,
                          "success": lambda: True, "failure": lambda: False})
    env = {cle: str(ev.evalue(v)) if "${{" in v else v for cle, v in env_verdict(blocs["convergence"]).items()}
    lignes, causes = c.juge(env)
    return {"resultats": resultats, "env": env, "nom_verdict": ev.evalue(conv["name"]) if "${{" in (conv["name"] or "") else conv["name"],
            "verdict_ok": resultats["convergence"] != "skipped" and not causes,
            "causes": causes, "lignes": lignes}


def ordre_topologique(graphe):
    vus, ordre = set(), []

    def visite(n):
        if n in vus:
            return
        vus.add(n)
        for b in graphe[n]["needs"]:
            visite(b)
        ordre.append(n)
    for n in graphe:
        visite(n)
    return ordre


# Scenarios nominaux : le verdict DOIT etre vert, avec le bon palier et le bon nom.
NOMINAUX = (
    (Scenario("qualification (dispatch par defaut)", "workflow_dispatch", "qualification", mode="construit"),
     contrat.QUALIFICATION),
    (Scenario("qualification (branche d'audit)", "push", ref="refs/heads/audit/p13-console-sync-ci-20261024",
              mode="construit"), contrat.QUALIFICATION),
    (Scenario("developpement, artefact reutilise", "push", mode="reutilise"), contrat.DEVELOPPEMENT),
    (Scenario("developpement, artefact construit", "push", mode="construit"), contrat.DEVELOPPEMENT),
    (Scenario("developpement par dispatch", "workflow_dispatch", "developpement", mode="reutilise"),
     contrat.DEVELOPPEMENT),
    (Scenario("developpement, pull_request", "pull_request", mode="reutilise"), contrat.DEVELOPPEMENT),
)

# Scenarios de panne : le verdict DOIT etre rouge.
PANNES = (
    Scenario("epreuve WPT en echec (dev)", "push", mode="reutilise", pannes=("wpt",)),
    Scenario("epreuve requise sautee (dev)", "push", mode="reutilise", pannes={"endurance-longue": "skipped"}),
    Scenario("epreuve requise sautee (qualification)", "workflow_dispatch", "qualification", mode="construit",
             pannes={"sites": "skipped"}),
    Scenario("epreuve requise annulee (dev)", "push", mode="reutilise", pannes={"cache": "cancelled"}),
    Scenario("primitives sautees (qualification)", "workflow_dispatch", "qualification", mode="construit",
             pannes={"primitives": "skipped"}),
    Scenario("smoke en echec : performance sautee (dev)", "push", mode="reutilise", pannes=("browser-host-smoke",)),
    Scenario("reutilisation en echec (dev)", "push", mode="reutilise", pannes=("reutilise",)),
    Scenario("provenance en echec : repli build (dev)", "push", mode="", pannes=("provenance",)),
    Scenario("build en echec (qualification)", "workflow_dispatch", "qualification", mode="construit",
             pannes=("build",)),
    Scenario("primitives en echec (qualification)", "workflow_dispatch", "qualification", mode="construit",
             pannes=("primitives",)),
    Scenario("qualification sur decision de reutilisation", "workflow_dispatch", "qualification", mode="reutilise"),
    Scenario("artefact en echec (qualification)", "workflow_dispatch", "qualification", mode="construit",
             pannes=("artefact",)),
)


def controle(texte: str, c=contrat) -> list[str]:
    """Erreurs : scenario nominal non vert, ou scenario de panne non rouge."""
    erreurs = []
    for sc, palier in NOMINAUX:
        try:
            r = simule(texte, sc, c)
        except SimulationErreur as exc:
            erreurs.append(f"{sc.nom} : {exc}")
            continue
        sautes = sorted(e.job for e in c.EPREUVES
                        if e.exigence[palier] == c.REQUIS and r["resultats"].get(e.job) != "success")
        if r["env"].get("BOUCHAUD_PALIER") != palier:
            erreurs.append(f"{sc.nom} : palier {r['env'].get('BOUCHAUD_PALIER')!r}, attendu {palier}")
        if r["nom_verdict"] != c.NOM_VERDICT[palier]:
            erreurs.append(f"{sc.nom} : verdict nomme {r['nom_verdict']!r}")
        if not r["verdict_ok"]:
            erreurs.append(f"{sc.nom} : verdict rouge ({','.join(r['causes'])}) ; requis non joues : {sautes}")
    for sc in PANNES:
        try:
            r = simule(texte, sc, c)
        except SimulationErreur as exc:
            erreurs.append(f"{sc.nom} : {exc}")
            continue
        if r["verdict_ok"]:
            erreurs.append(f"{sc.nom} : verdict VERT malgre la panne")
    return erreurs


def main() -> int:
    texte = WORKFLOW.read_text(encoding="utf-8")
    for sc, palier in NOMINAUX:
        r = simule(texte, sc)
        joues = sum(1 for e in contrat.EPREUVES if r["resultats"].get(e.job) == "success")
        print(f"SIMULATION {sc.nom} : palier={r['env'].get('BOUCHAUD_PALIER')} verdict="
              f"{'vert' if r['verdict_ok'] else 'ROUGE'} nom={r['nom_verdict']!r} epreuves_jouees={joues} "
              f"causes={r['causes']}")
    for sc in PANNES:
        r = simule(texte, sc)
        print(f"SIMULATION panne {sc.nom} : verdict={'VERT' if r['verdict_ok'] else 'rouge'} causes={r['causes']}")
    erreurs = controle(texte)
    for e in erreurs:
        print(f"SIMULATION_ECHEC {e}")
    return 1 if erreurs else 0


if __name__ == "__main__":
    import sys
    sys.exit(main())
