"""Identite du navigateur."""

VERSION = "2.0.0"
CODENAME = "Argonauta"
ENGINE = "Nautile Engine (Python)"
TAGLINE = "Navigateur Web ecrit en Python, du parseur HTML au compositeur."


def user_agent():
    return (
        "Mozilla/5.0 (compatible; Nautile/%s; +https://github.com/bcharthur/nautile-navigateur)"
        % VERSION
    )


def python_version():
    import sys

    return sys.version.split()[0]


def platform_name():
    import sys

    # Bouchaud OS s'annonce via son interpreteur embarque.
    if hasattr(sys, "implementation") and getattr(sys.implementation, "name", "") == "rustpython":
        return "Bouchaud OS (RustPython/WASM)"
    return getattr(sys, "platform", "inconnu")
