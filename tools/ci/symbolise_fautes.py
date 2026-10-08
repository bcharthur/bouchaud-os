#!/usr/bin/env python3
"""Symbolise les fautes des processus Ladybird d'un journal serie.

BOUCHAUD_SYMBOLISE_FAUTES_V1

    tools/ci/symbolise_fautes.py JOURNAL_SERIE DOSSIER_BINAIRES

Le noyau imprime, pour chaque faute utilisateur fatale :

    PROCESS_FAULT pid=19 reason=faute de page rip=0x400009209064 ... cr2=...

et, pour chaque exec, la base de chargement de l'image PIE :

    PERF_EXEC_PRET image=/usr/libexec/ladybird/WebContent pid=19 ... base=0x400000000000

L'adresse seule ne dit rien. Rapportee a la base et resolue par addr2line
contre le binaire EXACT qui a tourne (l'artefact du meme run, verifie par son
manifeste), elle nomme la fonction. Run 37585729384, WPT : WebContent faute
sur cr2=0xffff413f021ee480 -- la MEME adresse non canonique qu'aux runs
37518121906 et 405 : une valeur deterministe, qu'il faut localiser.

BOUCHAUD_PILE_DE_FAUTE_V1 : le RIP d'une faute volontaire (`ak_trap`, un
`VERIFY` ou un `MUST()` qui echoue) ne nomme que le piege. Le noyau publie
aussi `PROCESS_FAULT_PILE pid= base= adresses=` (les mots de la pile qui
tombent dans l'image) : ils sont symbolises ici, dans l'ordre ; et les lignes
qui precedent la faute -- `UNEXPECTED ERROR`, `VERIFICATION FAILED`,
`ASSERTION FAILED`, la trace d'AK -- sont reimprimees avec elle.

Ne juge rien : imprime une ligne `FAUTE_SYMBOLE` par faute et rend 0.
"""
import re
import shutil
import subprocess
import sys
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")
EXEC = re.compile(r"PERF_EXEC_PRET image=(\S+) pid=(\d+) .*?base=(0x[0-9a-f]+)")
EXECVE = re.compile(r"PERF_EXECVE .*?image=(\S+) pid=(\d+)")
PILE = re.compile(r"PROCESS_FAULT_PILE pid=(\d+) (?:image=(\S+) )?base=(0x[0-9a-f]+) rsp=(0x[0-9a-f]+) adresses=(\S+)")
PREFIXE_SERIE = re.compile(r"^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] ")
ERREUR = re.compile(r"UNEXPECTED ERROR|VERIFICATION FAILED|ASSERTION FAILED|Assertion .* failed|terminate called|panicked at")
# Releves periodiques : sans rapport avec une faute, ils noieraient le contexte.
BRUIT = re.compile(r"^\[(PERF-|PROC-STAT|SMP-SNAPSHOT)|^(FAULT_FILE_SNAPSHOT|CPU_CUMUL|CACHE_BALAYAGE|\[REGISTRE-)")
FAUTE = re.compile(r"PROCESS_FAULT pid=(\d+) reason=(.*?) rip=(0x[0-9a-f]+) rsp=(0x[0-9a-f]+) cr2=(0x[0-9a-f]+)(?: base=(0x[0-9a-f]+))?")
# /bo-navigateur est une copie de BouchaudBrowserHost.
ALIAS = {"bo-navigateur": "BouchaudBrowserHost"}


def outil():
    for nom in ("llvm-addr2line", "addr2line", "eu-addr2line"):
        chemin = shutil.which(nom)
        if chemin:
            return chemin
    return None


def main(argv):
    if len(argv) != 3:
        print(__doc__.strip().splitlines()[4], file=sys.stderr)
        return 0
    journal, dossier = Path(argv[1]), Path(argv[2])
    if not journal.is_file():
        return 0
    images, bases, fautes, piles, lignes = {}, {}, [], {}, []
    for ligne in journal.read_text(errors="replace").splitlines():
        ligne = PREFIXE_SERIE.sub("", ANSI.sub("", ligne)).replace("\r", "")
        lignes.append(ligne)
        if m := EXEC.search(ligne):
            images[m[2]] = m[1]
            bases[m[2]] = int(m[3], 16)
        elif m := EXECVE.search(ligne):
            images.setdefault(m[2], m[1])
        if m := FAUTE.search(ligne):
            fautes.append((m, len(lignes) - 1))
        if m := PILE.search(ligne):
            piles[m[1]] = (int(m[3], 16), [] if m[5] == "-" else [int(a, 16) for a in m[5].split(",")])
            # Un fils de fork n'a pas d'exec a lui : le noyau donne son image.
            if m[2] and m[2] != "?":
                images.setdefault(m[1], m[2])
    if not fautes:
        return 0
    a2l = outil()
    print()
    print(f"== fautes des processus, symbolisees ({len(fautes)}) ==")
    for f, indice in fautes[:12]:
        pid, raison, rip, cr2 = f[1], f[2], int(f[3], 16), f[5]
        image = images.get(pid, "?")
        nom = ALIAS.get(Path(image).name, Path(image).name)
        binaire = dossier / nom
        # La ligne de faute porte la base depuis BOUCHAUD_SYMBOLISE_FAUTES_V1 ;
        # sinon, celle de l'exec (absente pour les services du navigateur).
        base = int(f[6], 16) if f[6] else bases.get(pid)
        entete = f"FAUTE_SYMBOLE pid={pid} image={image} rip={rip:#x} cr2={cr2} raison={raison.strip()}"
        if base is None or not binaire.is_file():
            print(f"{entete} -> non symbolisable (base={'?' if base is None else hex(base)} binaire={'present' if binaire.is_file() else 'absent'})")
            continue
        decalage = rip - base
        if a2l is None:
            print(f"{entete} decalage={decalage:#x} -> addr2line absent")
            continue
        try:
            sortie = subprocess.run([a2l, "-f", "-C", "-i", "-e", str(binaire), hex(decalage)],
                                    capture_output=True, text=True, timeout=120).stdout.strip().splitlines()
        except (OSError, subprocess.TimeoutExpired) as erreur:
            print(f"{entete} decalage={decalage:#x} -> addr2line : {erreur}")
            continue
        # Paires (fonction, fichier:ligne), la plus interne d'abord (-i : inlines).
        paires = [f"{sortie[i]} ({sortie[i + 1] if i + 1 < len(sortie) else '?'})" for i in range(0, len(sortie), 2)]
        print(f"{entete} decalage={decalage:#x}")
        for p in paires[:6]:
            print(f"    {p}")
        pile_de_faute(a2l, binaire, base, piles.get(pid))
        contexte_de_faute(lignes, indice)
    return 0


def pile_de_faute(a2l, binaire, base, pile):
    """Symbolise les adresses de pile publiees par le noyau, dans l'ordre."""
    if not pile or a2l is None:
        return
    base_pile, adresses = pile
    if not adresses:
        print("    pile : aucune adresse de l'image")
        return
    # Une adresse de RETOUR pointe apres le `call` ; quand l'appele ne rend
    # pas la main (ak_verification_failed, ak_trap), ce `call` est la
    # derniere instruction de la fonction et l'adresse tombe hors d'elle.
    # On resout donc `adresse - 1`, comme un debogueur.
    decalages = [a - base_pile - 1 for a in adresses]
    try:
        sortie = subprocess.run([a2l, "-f", "-C", "-a", "-e", str(binaire)] + [hex(d) for d in decalages],
                                capture_output=True, text=True, timeout=300).stdout.splitlines()
    except (OSError, subprocess.TimeoutExpired) as erreur:
        print(f"    pile : addr2line : {erreur}")
        return
    print(f"    pile ({len(adresses)} adresses de l'image, de la plus recente a la plus ancienne) :")
    # Sans -i : trois lignes par adresse (0x..., fonction, fichier:ligne).
    for k in range(0, len(sortie) - 2, 3):
        print(f"    [{k // 3:2d}] {sortie[k + 1][:150]} ({sortie[k + 2]})")


def contexte_de_faute(lignes, indice):
    """Les messages d'erreur et les lignes utiles qui precedent la faute."""
    debut = max(0, indice - 400)
    erreurs = [l for l in lignes[debut:indice] if ERREUR.search(l)]
    if erreurs:
        print("    messages d'erreur avant la faute :")
        for l in erreurs[-6:]:
            print(f"      {l[:220]}")
    utiles = [l for l in lignes[debut:indice] if l.strip() and not BRUIT.search(l)]
    print("    contexte (dernieres lignes avant la faute) :")
    for l in utiles[-20:]:
        print(f"      {l[:220]}")


if __name__ == "__main__":
    sys.exit(main(sys.argv))
