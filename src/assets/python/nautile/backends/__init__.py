"""Backends de rendu.

Chacun consomme une `DisplayList` et l'affiche sur sa surface. Le moteur ne
sait rien d'eux : ajouter un backend n'exige aucune modification du pipeline.
"""

from .base import Canvas, render

__all__ = ["Canvas", "render", "available_backends", "pick"]


def available_backends():
    """Backends utilisables ici, du plus riche au plus simple."""
    found = []
    try:
        from .qt import HAVE_QT

        if HAVE_QT:
            found.append("qt")
    except ImportError:
        pass
    try:
        from .tk import HAVE_TK

        if HAVE_TK:
            found.append("tk")
    except ImportError:
        pass
    try:
        from .bouchaud import available

        if available():
            found.append("bouchaud")
    except ImportError:
        pass
    found.append("text")
    return found


def pick(preferred=None):
    """Choisit un backend : celui demande s'il marche, sinon le meilleur."""
    found = available_backends()
    if preferred and preferred in found:
        return preferred
    if preferred:
        raise RuntimeError(
            "backend « %s » indisponible ; disponibles : %s"
            % (preferred, ", ".join(found))
        )
    # Bouchaud OS passe avant Qt/Tk : dans l'OS, c'est le seul qui affiche.
    for candidate in ("bouchaud", "qt", "tk", "text"):
        if candidate in found:
            return candidate
    return "text"
