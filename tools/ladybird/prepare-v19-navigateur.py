#!/usr/bin/env python3
"""V19 — les fonctions de navigateur que le chrome M11 n'avait pas.

S'execute APRES `prepare-m11-chrome.py`, dont il suppose les ancres : le
chrome est deja copie dans `Services/WebContent/`, `PageClient.cpp` l'inclut
deja, et `bouchaud_m11_start()` existe deja.

Ce que ce script branche, et rien de plus :

  * le survol de lien -- `page_did_hover_link` / `page_did_unhover_link` --
    sur la bulle d'etat du chrome.

Pourquoi un script separe de `prepare-m11-chrome.py` : celui-la construit le
chrome, celui-ci lui donne ce que le MOTEUR sait et qu'il ignorait. Les deux
ensembles d'ancres n'ont pas la meme duree de vie -- les premieres sont notre
propre texte, les secondes sont du texte upstream qui peut bouger d'un SHA a
l'autre -- et les melanger rendrait illisible le message d'erreur qui dit
laquelle a lache.
"""

from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-v19-navigateur.py <ladybird-worktree>")

root = Path(sys.argv[1])


def substitute(path: Path, old: str, new: str, label: str) -> None:
    data = path.read_text()
    if new in data:
        return
    if old not in data:
        raise SystemExit(f"V19 : ancre introuvable ({label}) dans {path}")
    path.write_text(data.replace(old, new, 1))


def ensure_include(path: Path, include: str, anchor: str, label: str) -> None:
    data = path.read_text()
    if include in data:
        return
    if anchor not in data:
        raise SystemExit(f"V19 : ancre d'inclusion introuvable ({label}) dans {path}")
    path.write_text(data.replace(anchor, include + "\n" + anchor, 1))


page_cpp = root / "Services/WebContent/PageClient.cpp"

# `prepare-m11-chrome.py` l'a deja pose. Le redemander ici rend ce script
# lisible seul et ne coute rien : `ensure_include` ne fait rien si l'inclusion
# est la.
ensure_include(
    page_cpp,
    "#if defined(BOUCHAUD_PORT)\n#    include <WebContent/BouchaudChrome.h>\n#endif",
    "#include <WebContent/WebUIConnection.h>",
    "include BouchaudChrome PageClient.cpp",
)

# ---------------------------------------------------------------------------
# Survol de lien
# ---------------------------------------------------------------------------
#
# `async_did_hover_link` part vers le processus hote, qui sous Bouchaud ne
# dessine aucun chrome : la barre d'outils vit dans WebContent. Le message
# n'allait donc nulle part, et voir ou mene un lien avant de cliquer -- la
# seule defense qu'un navigateur offre contre un texte de lien qui ment --
# n'existait pas.
substitute(
    page_cpp,
    """void PageClient::page_did_hover_link(URL::URL const& url)
{
    client().async_did_hover_link(m_id, url);
}""",
    """void PageClient::page_did_hover_link(URL::URL const& url)
{
#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled() && BouchaudChrome::enabled()) {
        BouchaudChrome::set_survol_url(url.to_byte_string());
        return;
    }
#endif
    client().async_did_hover_link(m_id, url);
}""",
    "survol de lien",
)

substitute(
    page_cpp,
    """void PageClient::page_did_unhover_link()
{
    client().async_did_unhover_link(m_id);
}""",
    """void PageClient::page_did_unhover_link()
{
#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled() && BouchaudChrome::enabled()) {
        BouchaudChrome::clear_survol_url();
        return;
    }
#endif
    client().async_did_unhover_link(m_id);
}""",
    "fin de survol de lien",
)

print("Bouchaud V19 navigateur applique a", root)
