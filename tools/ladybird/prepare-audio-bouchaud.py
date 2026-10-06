#!/usr/bin/env python3
"""Installe la sortie audio Bouchaud de LibMedia : /dev/dsp (OSS).

BOUCHAUD_AUDIO_DSP_V1 (P8)

Sans libpulse, `Meta/CMake/audio.cmake` ne choisit aucun backend et LibMedia
joue tout sur la sortie NULLE. Ce preparateur :

  1. copie `tools/ladybird/audio/PlaybackStreamBouchaud.cpp` dans
     `Libraries/LibMedia/Audio/` (le source vit dans le depot, jamais ecrit
     ici) ;
  2. fait choisir `BOUCHAUD_OSS` a `audio.cmake` sous `BOUCHAUD_PORT` ;
  3. ajoute la branche `BOUCHAUD_OSS` a `Libraries/LibMedia/CMakeLists.txt`,
     a cote de PULSE / AUDIO_UNIT / WASAPI, avec `LIBMEDIA_AUDIO_BACKEND=1` --
     ce qui active le repli upstream sur la sortie nulle si /dev/dsp manque.

Idempotent ; fail-closed si une ancre upstream a change.
"""
import shutil
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[2]
SOURCE = RACINE / "tools/ladybird/audio/PlaybackStreamBouchaud.cpp"
MARQUEUR = "BOUCHAUD_AUDIO_DSP_V1"


def remplace_une_fois(chemin: Path, ancre: str, nouveau: str) -> None:
    texte = chemin.read_text(encoding="utf-8")
    if MARQUEUR in texte:
        return
    if texte.count(ancre) != 1:
        raise SystemExit(f"audio Bouchaud : ancre introuvable ou ambigue dans {chemin}")
    chemin.write_text(texte.replace(ancre, nouveau, 1), encoding="utf-8")


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: prepare-audio-bouchaud.py <arbre-ladybird>", file=sys.stderr)
        return 2
    arbre = Path(sys.argv[1]).resolve()

    shutil.copyfile(SOURCE, arbre / "Libraries/LibMedia/Audio/PlaybackStreamBouchaud.cpp")

    remplace_une_fois(
        arbre / "Meta/CMake/audio.cmake",
        "include_guard()\n",
        "include_guard()\n\n"
        f"# {MARQUEUR} : sur Bouchaud OS, /dev/dsp (OSS) -- tools/ladybird/audio/.\n"
        "if (BOUCHAUD_PORT)\n"
        '    set(LADYBIRD_AUDIO_BACKEND "BOUCHAUD_OSS")\n'
        "    return()\n"
        "endif()\n",
    )
    remplace_une_fois(
        arbre / "Libraries/LibMedia/CMakeLists.txt",
        "elseif (DEFINED LADYBIRD_AUDIO_BACKEND)\n",
        f'elseif (LADYBIRD_AUDIO_BACKEND STREQUAL "BOUCHAUD_OSS")\n'
        f"    # {MARQUEUR}\n"
        "    target_sources(LibMedia PRIVATE Audio/PlaybackStreamBouchaud.cpp)\n"
        "    target_compile_definitions(LibMedia PRIVATE LIBMEDIA_AUDIO_BACKEND=1)\n"
        "elseif (DEFINED LADYBIRD_AUDIO_BACKEND)\n",
    )
    print("audio Bouchaud : PlaybackStreamBouchaud installe (BOUCHAUD_OSS, /dev/dsp)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
