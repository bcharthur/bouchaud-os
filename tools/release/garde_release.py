#!/usr/bin/env python3
"""BOUCHAUD_RELEASE_V1 -- la garde d'une publication Trigkey : refuse ou laisse passer.

Lancee par .github/workflows/release-trigkey.yml AVANT toute construction et
avant que le secret de signature ne soit expose a un job. Fail closed :

  publier   --sha SHA40 --ref REF --version V --sequence N
            --runs runs.json --tags tags.txt
    - REF doit etre refs/heads/main (un push sur une branche ne publie rien,
      et un push sur main non plus : seule une demande manuelle publie) ;
    - chaque workflow de QUALIFICATION_REQUISE doit avoir une execution
      `success` sur CE SHA exact (la plus recente fait foi ; absente = refus,
      un saut par filtre de chemins n'est PAS une preuve : relancer le
      workflow a la main sur main) ;
    - au moins une cle de production engagee (src/kernel/update/cles_production.rs) ;
    - etiquette trigkey-s<N>-<V> jamais vue, sequence strictement superieure
      a toutes les precedentes, version jamais reutilisee.
    Imprime `GARDE_OK etiquette=... sequence_precedente=...`.

  promouvoir --etiquette T --tags tags.txt --prerelease true|false
    - l'etiquette existe et sa release est encore en `testing` (prerelease).

Code 2 + `GARDE_REFUSEE raison=...` sinon.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from fabrique_release import CLES_PRODUCTION, cles_production  # noqa: E402  (une seule lecture des cles)

QUALIFICATION_REQUISE = (
    "CI Fast",
    "Integration",
    "Non-regression Gate",
    "Reliability V3",
    "Unix & Filesystem Reliability",
)
ETIQUETTE_RE = re.compile(r"^trigkey-s([1-9][0-9]{0,18})-([A-Za-z0-9._+-]{1,64})$")


class Refus(Exception):
    pass


def etiquettes(tags: list[str]) -> list[tuple[int, str, str]]:
    r = []
    for t in tags:
        m = ETIQUETTE_RE.match(t.strip())
        if m:
            r.append((int(m.group(1)), m.group(2), t.strip()))
    return r


def qualification(runs: list[dict], sha: str) -> None:
    for nom in QUALIFICATION_REQUISE:
        siens = [r for r in runs if r.get("name") == nom and r.get("head_sha") == sha]
        if not siens:
            raise Refus(f"qualification_absente workflow={nom!r}")
        dernier = max(siens, key=lambda r: (r.get("run_number", 0), r.get("run_attempt", 0)))
        if dernier.get("status") != "completed":
            raise Refus(f"qualification_en_cours workflow={nom!r}")
        if dernier.get("conclusion") != "success":
            raise Refus(f"qualification_non_verte workflow={nom!r} conclusion={dernier.get('conclusion')}")


def publier(sha: str, ref: str, version: str, sequence: int, runs: list[dict], tags: list[str],
            cles_texte: str) -> tuple[str, int | None]:
    if ref != "refs/heads/main":
        raise Refus(f"ref_non_main ref={ref}")
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise Refus("sha_invalide")
    if not re.fullmatch(r"[A-Za-z0-9._+-]{1,64}", version):
        raise Refus("version_invalide")
    if not 0 < sequence < 10 ** 19:
        raise Refus("sequence_invalide")
    if not cles_production(cles_texte):
        raise Refus("aucune_cle_de_production_engagee")
    qualification(runs, sha)
    existantes = etiquettes(tags)
    etiquette = f"trigkey-s{sequence}-{version}"
    if any(t == etiquette for _, _, t in existantes):
        raise Refus(f"etiquette_existante {etiquette} (versions immuables)")
    if any(v == version for _, v, _ in existantes):
        raise Refus(f"version_deja_publiee {version}")
    precedente = max((s for s, _, _ in existantes), default=None)
    if precedente is not None and sequence <= precedente:
        raise Refus(f"sequence_non_croissante precedente={precedente} demandee={sequence}")
    return etiquette, precedente


def promouvoir(etiquette: str, tags: list[str], prerelease: str) -> None:
    if not ETIQUETTE_RE.match(etiquette):
        raise Refus("etiquette_invalide")
    if etiquette not in (t.strip() for t in tags):
        raise Refus(f"etiquette_inconnue {etiquette}")
    if prerelease != "true":
        raise Refus("deja_stable_ou_release_absente")


def main(argv: list[str]) -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sp = p.add_subparsers(dest="action", required=True)
    a = sp.add_parser("publier")
    a.add_argument("--sha", required=True)
    a.add_argument("--ref", required=True)
    a.add_argument("--version", required=True)
    a.add_argument("--sequence", type=int, required=True)
    a.add_argument("--runs", required=True)
    a.add_argument("--tags", required=True)
    b = sp.add_parser("promouvoir")
    b.add_argument("--etiquette", required=True)
    b.add_argument("--tags", required=True)
    b.add_argument("--prerelease", required=True)
    x = p.parse_args(argv[1:])
    try:
        tags = Path(x.tags).read_text().splitlines()
        if x.action == "publier":
            runs = json.loads(Path(x.runs).read_text())
            runs = runs.get("workflow_runs", runs) if isinstance(runs, dict) else runs
            etiquette, prec = publier(x.sha, x.ref, x.version, x.sequence, runs, tags,
                                      CLES_PRODUCTION.read_text(encoding="utf-8"))
            print(f"GARDE_OK etiquette={etiquette} sequence_precedente={'' if prec is None else prec}")
        else:
            promouvoir(x.etiquette, tags, x.prerelease)
            print(f"GARDE_OK promotion={x.etiquette}")
    except (Refus, OSError, ValueError) as e:
        print(f"GARDE_REFUSEE raison={e}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
