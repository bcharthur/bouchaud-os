#!/usr/bin/env python3
"""Publie le résultat réel du runner console, sans recalculer son verdict."""
from pathlib import Path
import re
import sys


def annotation(tag, status, raw):
    clean = re.sub(r'\x1b\[[0-9;]*m', '', raw)
    # Les erreurs causales sont en tête, les compteurs et le verdict en fin.
    if len(clean) > 18000:
        clean = clean[:8000] + '\n[journal complet dans l’artefact]\n' + clean[-10000:]
    body = f'exit={status}\n{clean}'
    body = body.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
    level = 'notice' if status == 0 else 'error'
    return f'::{level} title=P13 console {tag}::{body}'


if __name__ == '__main__':
    tag, code, path = sys.argv[1:]
    print(annotation(tag, int(code), Path(path).read_text(errors='replace')))
