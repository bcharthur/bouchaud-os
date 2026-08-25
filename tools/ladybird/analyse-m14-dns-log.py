#!/usr/bin/env python3
"""Analyse a Bouchaud OS Ladybird M13/M14 serial log."""

from __future__ import annotations

import re
import sys
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")

MARKERS = [
    "M9_RS_DNS_QUERY",
    "M13_DNS_SOCKET_OK",
    "M13_DNS_WRITE_BEGIN",
    "M13_DNS_WRITE_OK",
    "M13_DNS_TIMER_ARMED",
    "M13_DNS_TIMER_FIRED",
    "M14_DNS_RETRY_TX",
    "M13_DNS_NOTIFIER_FIRED",
    "M13_DNS_READY value=true",
    "M13_DNS_READY value=false",
    "M13_DNS_MESSAGE",
    "M13_DNS_PROMISE_RESOLVE",
    "M13_DNS_TIMEOUT",
    "M9_RS_DNS_RESOLU",
    "M9_RS_STATE id=0 DNSLookup -> RetrieveCookie",
    "M9_RS_STATE id=0 DNSLookup -> Error",
    "M9_NAVIGATION_COMMITTED",
    "M9_DOCUMENT_LOADED",
]


def count(text: str, marker: str) -> int:
    return text.count(marker)


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: analyse-m14-dns-log.py <serial-log>", file=sys.stderr)
        return 2

    path = Path(sys.argv[1])
    if not path.is_file():
        print(f"log introuvable: {path}", file=sys.stderr)
        return 2

    raw = path.read_text(errors="replace")
    text = ANSI.sub("", raw)

    print("=== Bouchaud Ladybird M14 DNS ===")
    for marker in MARKERS:
        n = count(text, marker)
        if n:
            print(f"{n:>3}  {marker}")

    print("\n=== Verdict ===")

    if "M9_RS_DNS_QUERY" not in text:
        print("La navigation n'atteint pas encore RequestServer::DNSLookup.")
        return 1

    if "M13_DNS_WRITE_OK" not in text:
        print("Le premier envoi DNS n'aboutit pas. Revenir au chemin socket/write M13.")
        return 1

    retry_count = count(text, "M14_DNS_RETRY_TX")
    timer_count = count(text, "M13_DNS_TIMER_FIRED")

    if timer_count and not retry_count:
        print("M14 n'est pas actif dans ce binaire: le timer tire, mais aucune retransmission M14 n'apparait.")
        print("Verifier le hook prepare-m14-dns-retry-fix.py et reconstruire l'artefact Ladybird du HEAD courant.")
        return 1

    if retry_count:
        print(f"M14 actif: {retry_count} retransmission(s) DNS ont atteint le chemin wire.")

    if "M13_DNS_PROMISE_RESOLVE" in text:
        print("La reponse DNS a ete parsée et la Promise LibDNS a ete resolue.")
        if "M9_RS_DNS_RESOLU" not in text:
            print("Prochaine frontiere: callback Promise / transition RequestServer apres LibDNS.")
            return 1
        if "M9_NAVIGATION_COMMITTED" in text or "M9_DOCUMENT_LOADED" in text:
            print("DNS passe et navigation distante progresse jusqu'au document. Continuer sur TLS/HTTP/rendu selon les marqueurs suivants.")
            return 0
        print("DNS passe. Prochaine frontiere: RetrieveCookie / Connect / TLS / Fetch.")
        return 0

    if "M13_DNS_NOTIFIER_FIRED" in text:
        if "M13_DNS_READY value=false" in text and "M13_DNS_READY value=true" not in text:
            print("Le notifier se declenche mais le socket est declare non lisible: frontiere poll/FIONREAD/readiness.")
            return 1
        if "M13_DNS_READY value=true" in text and "M13_DNS_MESSAGE" not in text:
            print("Le socket est lisible mais aucun message DNS n'est parse: frontiere recv/BufferedUDPSocket/parser.")
            return 1
        if "M13_DNS_MESSAGE" in text and "M13_DNS_PROMISE_RESOLVE" not in text:
            print("Un message DNS est parse mais la Promise ne se resout pas: verifier ID/pending lookup/DNSSEC.")
            return 1

    if retry_count and "M13_DNS_NOTIFIER_FIRED" not in text:
        if "M13_DNS_TIMEOUT" in text or "DNSLookup -> Error" in text:
            print("Le bug de retry est corrige, mais aucune reponse UDP n'atteint le notifier.")
            print("Prochaine frontiere: reception UDP Bouchaud OS -> demux -> poll/notifier LibCore.")
            return 1
        print("Les retries partent mais aucune notification de lecture n'apparait encore.")
        print("Attendre le timeout LibDNS ou inspecter la reception UDP/demux/poll.")
        return 1

    print("Trace insuffisante pour classer la frontiere suivante.")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
