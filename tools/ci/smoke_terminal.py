#!/usr/bin/env python3
"""Quand le banc BrowserHost a-t-il le droit de quitter sa boucle ?

BOUCHAUD_SMOKE_TERMINAL_V1

## Le defaut (Ladybird #382, job « ordre worker », bras blob)

Avec `BO_SMOKE_ATTEND_AB=1`, la boucle sortait des qu'elle voyait
`HOST_WORKER_AB_COMPLETE`. Or la page emet cette ligne AVANT ses verdicts :

    HOST_WORKER_AB rang=4 ...
    HOST_WORKER_AB_COMPLETE          <- la boucle sortait ici
    HOST_WORKER_HTTP_FUNCTIONAL OK
    HOST_WORKER_BLOB_FUNCTIONAL OK
    HOST_WORKER_FUNCTIONAL_GLOBAL OK pong
    HOST_WORKER_COLD_START_PERF ...
    HOST_SMOKE_OK ...

Le banc tuait la VM a T+48 s, puis declarait JAMAIS ATTEINTS les quatre
jalons fonctionnels -- qui n'avaient simplement pas encore traverse l'IPC et
la console serie -- et concluait « QEMU s'est arrete de lui-meme » parce que
`ab_complete` n'avait pas de branche dans son diagnostic. Les quatre mesures
A/B de chaque bras etaient pourtant la (ORDRE_WORKER_ANALYSE_OK).

`HOST_WORKER_AB_FAIL` avait le meme defaut, et la sortie `rendu` du smoke
principal un cousin : elle quittait sur `HOST_SMOKE_` sans attendre que la
poignee de main de surface ait conclu.

## La regle

Une sortie n'a lieu que sur un verdict TERMINAL, c'est-a-dire une ligne que
la page n'ecrit qu'apres toutes celles que le banc va juger :

  * `BO_SMOKE_ATTEND_AB=1` : `HOST_WORKER_AB_VERDICT_COMPLETE`, emise apres
    `HOST_SMOKE_*` (donc apres FUNCTIONAL, GLOBAL, COLD_START_PERF), que la
    matrice ait reussi ou non ;
  * sinon : document charge, premiere trame, `HOST_SMOKE_*` ;
  * et dans les deux cas, la surface a conclu (`surface_conclue`).

`HOST_WORKER_AB_COMPLETE` et `HOST_WORKER_AB_FAIL` restent des INFORMATIONS
(la matrice est mesuree / ne l'est pas) ; ils ne terminent plus rien.

Ce module ne decide que de la SORTIE. Ce qui est rouge reste rouge : un rang
manquant, un worker qui ne repond pas, un terminal qui ne vient jamais
(plafond ou silence), une VM morte avant le terminal.
"""
import sys

TERMINAL_AB = "HOST_WORKER_AB_VERDICT_COMPLETE"
SMOKE = "HOST_SMOKE_"


def ancienne_regle(texte, attend_ab, document_vu, trame_vue):
    """La regle d'avant #382, gardee pour que le test la montre en defaut."""
    if attend_ab:
        if "HOST_WORKER_AB_COMPLETE" in texte:
            return "ab_complete"
        if "HOST_WORKER_AB_FAIL" in texte:
            return "ab_echec"
    if document_vu and trame_vue and SMOKE in texte:
        return "rendu"
    return None


def terminal(texte, attend_ab, document_vu, trame_vue, surface_conclue):
    """Rend le verdict de sortie, ou None pour continuer d'observer."""
    if not surface_conclue:
        return None
    if attend_ab:
        return "ab_verdict_complete" if TERMINAL_AB in texte else None
    if document_vu and trame_vue and SMOKE in texte:
        return "rendu"
    return None


# ---------------------------------------------------------------------------
# Banc hote : livraison progressive du journal, sondage toutes les 2 s.
# ---------------------------------------------------------------------------

PAGE_AB = [
    (20, "[ladybird-bouchaud] M11_DOCUMENT_LOADED"),
    (21, "[ladybird-bouchaud] BROWSER_HOST_M11_FRAME_PRESENTED page=1"),
    (40, "HOST_WORKER_AB rang=1 origine=blob repond=1 ms=1964"),
    (42, "HOST_WORKER_AB rang=2 origine=http repond=1 ms=1753"),
    (44, "HOST_WORKER_AB rang=3 origine=blob repond=1 ms=1632"),
    (46, "HOST_WORKER_AB rang=4 origine=http repond=1 ms=2169"),
    (46, "HOST_WORKER_AB_COMPLETE ordre=blob count=4 matrice=blob,http,blob,http"),
    # -- delai de livraison : IPC WebContent -> console -> serie --
    (49, "HOST_WORKER_HTTP_FUNCTIONAL OK successes=2/2"),
    (49, "HOST_WORKER_BLOB_FUNCTIONAL OK successes=2/2"),
    (49, "HOST_WORKER_FUNCTIONAL_GLOBAL OK pong http=1 blob=1"),
    (49, "HOST_WORKER_COLD_START_PERF OK ms=1964 origine=blob budget=30000"),
    (49, "HOST_SMOKE_OK canvas=1 worker=1 image=1 frame=1"),
    (49, "HOST_WORKER_AB_VERDICT_COMPLETE ordre=blob rangs=4/4 fonctionnel=1 smoke=1"),
]

JALONS_FONCTIONNELS = (
    "HOST_WORKER_HTTP_FUNCTIONAL OK",
    "HOST_WORKER_BLOB_FUNCTIONAL OK",
    "HOST_WORKER_FUNCTIONAL_GLOBAL OK pong",
    "HOST_SMOKE_OK",
)


def joue(page, regle, attend_ab=True, surface_a=0, sonde=2, plafond=120, qemu_meurt_a=None):
    """Rend (instant de sortie, verdict, journal vu a la sortie)."""
    t = 0
    while t <= plafond:
        if qemu_meurt_a is not None and t >= qemu_meurt_a:
            vu = "".join(l + "\n" for a, l in page if a <= qemu_meurt_a)
            return t, "qemu_morte", vu
        vu = "".join(l + "\n" for a, l in page if a <= t)
        doc = "M11_DOCUMENT_LOADED" in vu
        trame = "FRAME_PRESENTED" in vu
        if regle is ancienne_regle:
            v = ancienne_regle(vu, attend_ab, doc, trame)
        else:
            v = terminal(vu, attend_ab, doc, trame, t >= surface_a)
        if v:
            return t, v, vu
        t += sonde
    return t, "plafond", "".join(l + "\n" for a, l in page if a <= plafond)


def manquants(vu):
    return [j for j in JALONS_FONCTIONNELS if j not in vu]


def _cas(nom, obtenu, attendu):
    bon = obtenu == attendu
    print(f"  {'ok   ' if bon else 'ECHEC'} {nom} -> {obtenu}")
    return 0 if bon else 1


def autotest():
    echecs = 0
    print("smoke/terminal : sortie de boucle du banc BrowserHost")

    # 1. LE CAS #382 : l'ancienne regle sort a AB_COMPLETE, avant les verdicts.
    t, v, vu = joue(PAGE_AB, ancienne_regle)
    echecs += _cas("ancienne regle : sortie sur AB_COMPLETE", (t, v), (46, "ab_complete"))
    echecs += _cas("ancienne regle : 4 jalons fonctionnels perdus", len(manquants(vu)), 4)

    # 2. La nouvelle attend le vrai terminal ; rien ne manque a la sortie.
    t, v, vu = joue(PAGE_AB, terminal)
    echecs += _cas("nouvelle regle : sortie sur le terminal", (t, v), (50, "ab_verdict_complete"))
    echecs += _cas("nouvelle regle : aucun jalon perdu", manquants(vu), [])

    # 3. Terminal JAMAIS emis (page bloquee apres AB_COMPLETE) : pas de sortie
    #    anticipee ; la boucle va au plafond, et le bras sera rouge.
    page = [p for p in PAGE_AB if "VERDICT_COMPLETE" not in p[1]]
    echecs += _cas("terminal absent -> plafond (rouge)", joue(page, terminal)[1], "plafond")

    # 4. Un worker qui ne repond pas : le terminal vient, la sortie a lieu,
    #    et le jalon fonctionnel manque VRAIMENT -- rouge, pour la bonne raison.
    page = [(a, l.replace("BLOB_FUNCTIONAL OK successes=2/2", "BLOB_FUNCTIONAL FAIL successes=0/2")
             .replace("FUNCTIONAL_GLOBAL OK pong", "FUNCTIONAL_GLOBAL FAIL")
             .replace("HOST_SMOKE_OK", "HOST_SMOKE_FAIL"))
            for a, l in PAGE_AB]
    t, v, vu = joue(page, terminal)
    echecs += _cas("worker blob muet -> sortie terminale, jalons rouges",
                   (v, manquants(vu)), ("ab_verdict_complete",
                                        ["HOST_WORKER_BLOB_FUNCTIONAL OK",
                                         "HOST_WORKER_FUNCTIONAL_GLOBAL OK pong", "HOST_SMOKE_OK"]))

    # 5. AB_FAIL (matrice incomplete) n'est plus terminal : on attend le
    #    terminal, qui porte rangs=3/4.
    page = [(a, l) for a, l in PAGE_AB if "rang=4" not in l and "AB_COMPLETE" not in l]
    page.append((46, "HOST_WORKER_AB_FAIL ordre=blob rang=4 origine=http phase=rang_3_termine raison=matrice_incomplete"))
    page = [(a, l.replace("rangs=4/4", "rangs=3/4")) for a, l in page]
    page.sort(key=lambda x: x[0])
    t, v, vu = joue(page, ancienne_regle)
    echecs += _cas("ancienne regle : sortie sur AB_FAIL, verdicts perdus", (v, len(manquants(vu))), ("ab_echec", 4))
    t, v, vu = joue(page, terminal)
    echecs += _cas("nouvelle regle : AB_FAIL attend le terminal", (v, "rangs=3/4" in vu), ("ab_verdict_complete", True))

    # 6. QEMU meurt AVANT le terminal : verdict qemu_morte, jamais un succes.
    echecs += _cas("VM morte avant le terminal -> qemu_morte",
                   joue(PAGE_AB, terminal, qemu_meurt_a=47)[1], "qemu_morte")

    # 7. La surface n'a pas conclu : meme le terminal ne fait pas sortir.
    t, v, _ = joue(PAGE_AB, terminal, surface_a=60)
    echecs += _cas("surface en cours -> sortie apres sa conclusion", (t, v), (60, "ab_verdict_complete"))

    # 8. Smoke principal (sans A/B) : HOST_SMOKE_ + surface conclue.
    t, v, _ = joue(PAGE_AB, terminal, attend_ab=False, surface_a=55)
    echecs += _cas("smoke principal : rendu apres la surface", (t, v), (56, "rendu"))

    if echecs:
        print(f"smoke/terminal : {echecs} cas en echec", file=sys.stderr)
        return 1
    print("SMOKE_TERMINAL_OK")
    return 0


def main():
    if len(sys.argv) >= 2 and sys.argv[1] == "--test":
        return autotest()
    # smoke_terminal.py <journal> <attend_ab 0|1> <document_vu 0|1> <trame_vue 0|1> <surface_conclue 0|1>
    if len(sys.argv) != 6:
        print("usage: smoke_terminal.py <journal> <attend_ab> <document> <trame> <surface_conclue>",
              file=sys.stderr)
        return 2
    try:
        with open(sys.argv[1], "rb") as flux:
            texte = flux.read().decode("utf-8", "replace")
    except OSError:
        texte = ""
    drapeaux = [a == "1" for a in sys.argv[2:6]]
    print(terminal(texte, *drapeaux) or "")
    return 0


if __name__ == "__main__":
    sys.exit(main())
