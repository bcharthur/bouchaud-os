#!/usr/bin/env python3
"""Installe le bac a sable Bouchaud des services Ladybird (BOUCHAUD_SANDBOX_V1).

Le noyau confine chaque service des l'exec, par profil
(`src/kernel/security/profile.rs`). Chaque service le VERIFIE avant de traiter
la moindre donnee venue du reseau, et s'arrete s'il ne l'est pas
(`tools/ladybird/sandbox/BouchaudConfinement.h`).

Ce script COPIE les sources -- il ne patche rien : les `CMakeLists.txt` les
designent deja (prepare-browser-source.py pour WebContent,
prepare-browser-runtime-link.py pour les autres). Il refuse de continuer si
une cible Bouchaud construit encore une implementation `Unimplemented`.
"""
from pathlib import Path
import shutil
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-sandbox-bouchaud.py <ladybird-worktree>")

root = Path(sys.argv[1]).resolve()
ici = Path(__file__).resolve().parent / "sandbox"
services = root / "Services"

COPIES = {
    "BouchaudConfinement.h": services / "BouchaudConfinement.h",
    "RendererSandboxBouchaud.cpp": services / "RendererSandboxBouchaud.cpp",
    "ImageDecoderSandboxBouchaud.cpp": services / "ImageDecoder/SandboxBouchaud.cpp",
    "CompositorSandboxBouchaud.cpp": services / "Compositor/SandboxBouchaud.cpp",
    "RequestServerSandboxBouchaud.cpp": services / "RequestServer/SandboxBouchaud.cpp",
}
for source, cible in COPIES.items():
    shutil.copyfile(ici / source, cible)

ATTENDU = {
    "WebContent/CMakeLists.txt": "../RendererSandboxBouchaud.cpp",
    "WebWorker/CMakeLists.txt": "../RendererSandboxBouchaud.cpp",
    "ImageDecoder/CMakeLists.txt": "SandboxBouchaud.cpp",
    "Compositor/CMakeLists.txt": "SandboxBouchaud.cpp",
    "RequestServer/CMakeLists.txt": "SandboxBouchaud.cpp",
}
for cmake, fichier in ATTENDU.items():
    texte = (services / cmake).read_text()
    branche = texte[texte.find("if (BOUCHAUD_PORT)"):][:200]
    if fichier not in branche or "Unimplemented" in branche:
        raise SystemExit(f"bac a sable Bouchaud : {cmake} ne construit pas {fichier} sous BOUCHAUD_PORT")

print(f"bac a sable Bouchaud installe : {len(COPIES)} fichiers, {len(ATTENDU)} services")
