"""Nautile — navigateur Web ecrit en Python.

Le pipeline complet vit dans ce paquet :

    URL -> reseau -> HTML -> DOM -> CSS -> style -> layout
        -> liste d'affichage -> backend de rendu

Usage courant :

    from nautile import Browser
    navigateur = Browser(width=1024, height=768)
    page = navigateur.navigate("https://example.com/")
    print(page.title)
"""

from .browser import Browser, Page, Tab
from .version import ENGINE, VERSION

__version__ = VERSION

__all__ = ["Browser", "Page", "Tab", "VERSION", "ENGINE"]
