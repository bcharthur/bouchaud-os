#!/usr/bin/env python3
"""Quand faut-il capturer l'ecran pour prouver que la mire est affichee ?

BOUCHAUD_C41_CAPTURE_VIVANTE, BOUCHAUD_SURFACE_POIGNEE_DE_MAIN_V1

La preuve visuelle se prend pendant que QEMU vit (run #347 : la capture tentee
apres la mort de la machine rendait `moniteur_muet`). Ce module decide QUAND,
et c'est lui qu'on met en echec sur des journaux synthetiques.

## Ce qui ne marchait pas (run 36836362480, #381)

L'ancienne regle : au marqueur `HOST_SURFACE_MIRE_INSEREE`, retenir la
derniere trame vue (8) et capturer des que la trame 8 + 2 parait. Elle
supposait que « toute trame posterieure au marqueur contient la mire ». Faux,
pour trois raisons independantes :

  1. le marqueur est ecrit par le SCRIPT, avant l'etape de rendu qui peint
     son DOM -- il precede la peinture qu'il pretend borner ;
  2. le vert passe par `background-image`, que la page n'attendait pas ;
  3. la capture de WebContent est a UN SEUL VOL (tools/ladybird/
     prepare-repaint.py, 2b) : celle qui revient juste apres le marqueur a pu
     etre lancee avant lui ;

et une quatrieme, en aval : `BROWSER_HOST_M11_TRAME` est ecrit quand
WebContent a REMIS la trame au bureau, pas quand le bureau l'a composee sur
le framebuffer que `screendump` lit.

La capture du #381 l'a dit en pixels : bleu 8704, rouge 512, vert 0. La page
porte 512 pixels rouges et 512 bleus HORS de la mire (la meme page sous
Chromium : rouge 4608, vert 4096, bleu 8704) : l'aplat bleu etait peint, le
rouge et le vert ne l'etaient pas encore. Une mire en cours de peinture,
capturee 312 ms apres le marqueur -- et les trames 11 a 17 ont suivi.

## La poignee de main

Quatre maillons, chacun prouve par une ligne du journal :

  A. la page ecrit `JS_CONSOLE log HOST_SURFACE_MIRE_PEINTE battement=trames`
     apres le DECODAGE des trois ressources et DEUX etapes de rendu
     (`requestAnimationFrame` x2) : une peinture contenant la mire a eu lieu
     avant cette ligne. `battement=delai` n'est pas une preuve : verdict
     `non_prouvee`, jamais un succes ;
  B. `seq_ancre` = derniere `BROWSER_HOST_M11_TRAME` AVANT l'ancre. Les deux
     lignes sont ecrites par `outln` dans le MEME processus WebContent : leur
     ordre dans le journal est l'ordre d'execution. La ligne `(js log)` de
     `dbgln` part sur un autre flux et n'est PAS utilisee. Capture a un seul
     vol : au plus une capture lancee avant l'ancre revient apres elle
     (`seq_ancre + 1`) ; `seq_ancre + 2` a ete lancee apres la fin de celle-la,
     donc apres l'ancre, donc apres la peinture ;
  C. `GUI_COMPOSITION_NAVIGATEUR pompe_t_ms=P` (src/gui/window_manager.rs) :
     si `P > t` de la trame retenue, le `FrameReady` de cette trame a ete lu
     par ce tour du bureau ou un precedent, et cette composition a laisse sur
     le framebuffer cette trame ou une plus recente. Meme horloge des deux
     cotes (`clock_gettime(MONOTONIC)` = `timer::monotonic_ns`) ;
  D. alors seulement : `screendump`.

## Si la mire n'y est pas

On ne conclut pas sur une capture. On garde la preuve (seq, t, pixels) et on
regarde au plus `SUIVANTES` presentations de plus, chacune NOUVELLE (seq
strictement superieur) et certifiee par sa propre composition. Aucune relance
au temps : une tentative n'existe que parce qu'une trame nouvelle a ete
composee.

  * mire complete a la premiere tentative     -> `ok`
  * mire complete a la tentative n > 1        -> `latence_publication`
    (n et l'ecart de trames sont dans le verdict : un retard mesure, pas un
    succes muet)
  * jamais complete en 1 + SUIVANTES tentatives -> `absente` : vrai echec.
"""
import re
import sys

# BOUCHAUD_UI_V1 : la console de la page sort par le navigateur
# (`[LB:JS] onglet=N log ...`) et la trame presentee par la fenetre
# (`[LB:FRAME] onglet=N seq=N t=T`). Les anciens journaux (`JS_CONSOLE log`,
# `BROWSER_HOST_M11_TRAME page=N`) se relisent encore : un journal archive doit
# rester analysable.
CONSOLE = r"(?:JS_CONSOLE|\[LB:JS\] onglet=\d+) log "
ANCRE = re.compile(
    CONSOLE + r"HOST_SURFACE_MIRE_PEINTE battement=(\w+)(?: decodees=(\d)/3)?")
INSEREE = re.compile(CONSOLE + r"HOST_SURFACE_MIRE_INSEREE")
TRAME = re.compile(r"(?:BROWSER_HOST_M11_TRAME page|\[LB:FRAME\] onglet)=\d+ seq=(\d+)(?: t=(\d+))?")
COMPOSITION = re.compile(r"GUI_COMPOSITION_NAVIGATEUR pompe_t_ms=(\d+)")
MARGE = 2
SUIVANTES = 3

# L'ancienne regle, gardee pour que le test la montre en defaut.
ANCIEN_MARQUEUR = "HOST_SURFACE_MIRE_INSEREE"
ANCIENNE_TRAME = re.compile(r"(?:BROWSER_HOST_M11_TRAME page|\[LB:FRAME\] onglet)=\d+ seq=(\d+)")


def ancienne_regle(texte):
    """(seq_insertion, seq_voulue, pret) de la version d'avant #381."""
    position = texte.find(ANCIEN_MARQUEUR)
    if position < 0:
        return (None, None, False)
    avant = [int(m.group(1)) for m in ANCIENNE_TRAME.finditer(texte, 0, position)]
    seq_insertion = max(avant) if avant else 0
    vues = [int(m.group(1)) for m in ANCIENNE_TRAME.finditer(texte)]
    return (seq_insertion, seq_insertion + MARGE, max(vues, default=0) >= seq_insertion + MARGE)


def lis(texte):
    trames = [(m.start(), int(m.group(1)), int(m.group(2)) if m.group(2) else None)
              for m in TRAME.finditer(texte)]
    compositions = [(m.start(), int(m.group(1))) for m in COMPOSITION.finditer(texte)]
    ancre = ANCRE.search(texte)
    return trames, compositions, ancre


def decide(texte, tentatives, fin=False):
    """Rend un dict : `action` in {attendre, capturer, conclure}.

    `tentatives` : liste de (seq, resultat) deja capturees, resultat in
    {ok, absente, capture_vide, moniteur_muet}. `fin` : QEMU ne vit plus,
    plus aucune ligne ne viendra.
    """
    trames, compositions, ancre = lis(texte)
    rendu = {"action": "attendre", "verdict": "", "seq": -1, "t_trame": -1,
             "pompe": -1, "seq_ancre": -1, "voulue": -1, "tentative": len(tentatives) + 1,
             "decodees": -1}

    def conclut(verdict):
        rendu["action"] = "conclure"
        rendu["verdict"] = verdict
        return rendu

    if ancre is None:
        return conclut("non_posee") if fin else rendu
    rendu["decodees"] = int(ancre.group(2)) if ancre.group(2) else -1
    if ancre.group(1) != "trames":
        return conclut("non_prouvee")

    seq_ancre = max((s for p, s, _ in trames if p < ancre.start()), default=0)
    rendu["seq_ancre"] = seq_ancre
    rendu["voulue"] = seq_ancre + MARGE

    # Bilan des tentatives deja faites.
    if tentatives:
        derniere_seq, dernier = tentatives[-1]
        if dernier == "ok":
            if len(tentatives) == 1:
                return conclut("ok")
            rendu["seq"] = derniere_seq
            rendu["verdict"] = (f"latence_publication tentatives={len(tentatives)}"
                                f" trames={derniere_seq - tentatives[0][0]}")
            rendu["action"] = "conclure"
            return rendu
        if dernier in ("moniteur_muet", "capture_vide"):
            # Une panne du BANC n'est pas une absence de mire.
            return conclut(dernier)
        if len(tentatives) >= 1 + SUIVANTES:
            return conclut("absente")

    plancher = seq_ancre + MARGE if not tentatives else tentatives[-1][0] + 1
    candidates = sorted((s, t) for p, s, t in trames if s >= plancher and p > ancre.start())
    if not candidates:
        if fin:
            return conclut("sans_trame_posterieure" if not tentatives else "absente")
        return rendu
    seq, t = candidates[0]
    rendu["seq"], rendu["t_trame"] = seq, (t if t is not None else -1)
    if t is None:
        # Un journal sans date de trame ne permet pas le maillon C.
        return conclut("trame_non_datee")
    certifiee = [pompe for _, pompe in compositions if pompe > t]
    if not certifiee:
        if fin:
            return conclut("non_composee" if not tentatives else "absente")
        return rendu
    rendu["pompe"] = certifiee[0]
    rendu["action"] = "capturer"
    return rendu


# ---------------------------------------------------------------------------
# Banc hote : un pipeline simule, l'ancienne regle et la nouvelle.
# ---------------------------------------------------------------------------

class Pipeline:
    """Modele discret du chemin page -> WebContent -> bureau -> framebuffer.

    * la page insere la mire a `t_insertion` ; la peinture COMPLETE n'a lieu
      qu'a `t_peinture` (decodage, `background-image`) ;
    * WebContent capture a un seul vol : une capture lancee a `d` rend la trame
      a `d + duree` ; son CONTENU est l'etat peint a `d` ;
    * le bureau compose une trame remise a `r` au plus tot a `r + latence_wm` ;
    * `jamais` : la mire n'est jamais peinte (vrai defaut de rendu).
    """

    def __init__(self, t_insertion, t_peinture, duree, latence_wm, captures,
                 jamais=False, t_ancre=None):
        self.lignes = []
        self.ecran = []  # (instant, contenu) des compositions
        remises = []     # (instant de remise, contenu)
        for seq, depart in enumerate(captures, 1):
            fin = depart + duree
            contenu = "mire" if (not jamais and depart >= t_peinture) else "ancienne"
            remises.append((fin, contenu))
            self.lignes.append((fin, f"[LB:FRAME] onglet=1 seq={seq} t={fin} degat=0,0 10x10 zone=10x10 present_us=1"))
        # Le bureau pompe `latence_wm` apres chaque remise ; une composition
        # montre la DERNIERE trame remise avant sa pompe -- l'invariant que
        # `temoin_composition` publie.
        for n, (fin, _) in enumerate(remises, 1):
            pompe = fin + latence_wm
            contenu = [c for r, c in remises if r < pompe][-1]
            self.ecran.append((pompe, contenu))
            self.lignes.append((pompe + 1, f"GUI_COMPOSITION_NAVIGATEUR pompe_t_ms={pompe}"
                                           f" fin_t_ms={pompe + 1} n={n}"))
        self.lignes.append((t_insertion, "[LB:JS] onglet=1 log HOST_SURFACE_MIRE_INSEREE largeur=256 hauteur=64"))
        # La page pose son ancre deux etapes de rendu apres ce qu'elle CROIT
        # etre la peinture complete ; `t_ancre` permet de la tromper.
        ancre = t_peinture + 2 if t_ancre is None else t_ancre
        self.lignes.append((ancre, "[LB:JS] onglet=1 log HOST_SURFACE_MIRE_PEINTE battement=trames decodees=3/3"))
        self.lignes.sort(key=lambda x: x[0])

    def journal_jusqua(self, instant):
        return "".join(l + "\n" for t, l in self.lignes if t <= instant)

    def ecran_a(self, instant):
        visibles = [c for t, c in self.ecran if t <= instant]
        return visibles[-1] if visibles else "vide"

    def instants(self):
        return sorted({t for t, _ in self.lignes})


def joue_ancienne(p):
    """Capture des que l'ancienne regle dit `pret` ; rend le contenu vu."""
    for instant in p.instants():
        _, _, pret = ancienne_regle(p.journal_jusqua(instant))
        if pret:
            return p.ecran_a(instant)
    return "jamais_capture"


def joue_nouvelle(p):
    tentatives = []
    for instant in p.instants():
        while True:
            d = decide(p.journal_jusqua(instant), tentatives)
            if d["action"] != "capturer":
                break
            vu = p.ecran_a(instant)
            tentatives.append((d["seq"], "ok" if vu == "mire" else "absente"))
        if d["action"] == "conclure":
            return d["verdict"]
    return decide(p.journal_jusqua(10 ** 9), tentatives, fin=True)["verdict"]


def _cas(nom, obtenu, attendu):
    bon = obtenu == attendu if not callable(attendu) else attendu(obtenu)
    print(f"  {'ok   ' if bon else 'ECHEC'} {nom} -> {obtenu}")
    return 0 if bon else 1


def autotest():
    echecs = 0
    print("surface/declencheur : regle et poignee de main")

    # 1. LE JOURNAL REEL DU #381 (lignes utiles, dans leur ordre).
    reel = (
        "BROWSER_HOST_M11_TRAME page=1 seq=8 t=23280\n"
        "23.607 WebContent(18): (js log) \"HOST_SURFACE_MIRE_INSEREE largeur=256 hauteur=64\"\n"
        "[ladybird-bouchaud] JS_CONSOLE log HOST_SURFACE_MIRE_INSEREE largeur=256 hauteur=64\n"
        "BROWSER_HOST_M11_TRAME page=1 seq=9 t=23724\n"
        "BROWSER_HOST_M11_TRAME page=1 seq=10 t=23919\n"
    )
    echecs += _cas("#381 : l'ancienne regle declenche a la trame 10",
                   ancienne_regle(reel), (8, 10, True))
    echecs += _cas("#381 : la nouvelle attend une ancre de PEINTURE",
                   decide(reel, [])["action"], "attendre")

    # 2. L'ancre posee, la trame ancre+2 arrivee, mais pas encore composee.
    j = reel + "[x] JS_CONSOLE log HOST_SURFACE_MIRE_PEINTE battement=trames decodees=3/3\n"
    j += "BROWSER_HOST_M11_TRAME page=1 seq=11 t=26209\nBROWSER_HOST_M11_TRAME page=1 seq=12 t=27247\n"
    echecs += _cas("ancre a 10 : cible 12, pas de composition -> attendre",
                   (decide(j, [])["voulue"], decide(j, [])["action"]), (12, "attendre"))
    # Une composition ANTERIEURE a la trame ne certifie rien.
    j2 = j + "GUI_COMPOSITION_NAVIGATEUR pompe_t_ms=27247 fin_t_ms=27250 n=9\n"
    echecs += _cas("composition a pompe = t de la trame : refusee (stricte)",
                   decide(j2, [])["action"], "attendre")
    j3 = j2 + "GUI_COMPOSITION_NAVIGATEUR pompe_t_ms=27248 fin_t_ms=27260 n=10\n"
    echecs += _cas("composition posterieure : capturer la trame 12",
                   (decide(j3, [])["action"], decide(j3, [])["seq"]), ("capturer", 12))
    echecs += _cas("tentative 1 ok -> ok", decide(j3, [(12, "ok")])["verdict"], "ok")
    echecs += _cas("tentative 1 absente, aucune trame nouvelle -> attendre",
                   decide(j3, [(12, "absente")])["action"], "attendre")
    echecs += _cas("... et QEMU meurt -> absente (vrai echec)",
                   decide(j3, [(12, "absente")], fin=True)["verdict"], "absente")
    echecs += _cas("battement=delai -> non_prouvee, jamais ok",
                   decide(reel + "JS_CONSOLE log HOST_SURFACE_MIRE_PEINTE battement=delai\n", [])["verdict"],
                   "non_prouvee")
    echecs += _cas("panne du banc -> moniteur_muet, pas absente",
                   decide(j3, [(12, "moniteur_muet")])["verdict"], "moniteur_muet")
    echecs += _cas("sans ancre a la fin -> non_posee", decide(reel, [], fin=True)["verdict"], "non_posee")

    # 3. LE CAS INJECTE : DOM pret, ancienne surface encore presentee.
    #    Insertion a 1000, peinture complete a 1450 ; captures de 200 ms a un
    #    seul vol ; le bureau compose 30 ms apres la remise.
    captures = [900, 1100, 1300, 1500, 1700, 1900]
    p = Pipeline(1000, 1450, 200, 30, captures)
    echecs += _cas("injecte : l'ANCIENNE regle capture une surface perimee (faux negatif)",
                   joue_ancienne(p), "ancienne")
    echecs += _cas("injecte : la NOUVELLE capture la mire", joue_nouvelle(p), "ok")

    # 4. Retard de composition du bureau plus long qu'une trame : la nouvelle
    #    regle ne capture qu'apres la composition datee.
    p = Pipeline(1000, 1450, 200, 350, captures)
    echecs += _cas("bureau lent : la nouvelle regle reste juste", joue_nouvelle(p), "ok")

    # 5. Vrai defaut : la mire n'est JAMAIS peinte. Aucune tolerance ne doit
    #    le transformer en succes.
    p = Pipeline(1000, 1450, 200, 30, captures + [2100, 2300, 2500], jamais=True)
    echecs += _cas("mire jamais peinte -> absente (borne a 1+3 tentatives)",
                   joue_nouvelle(p), "absente")

    # 6. Rendu tardif mais reel : complet seulement deux presentations apres la
    #    cible. Classe et mesure, pas confondu avec `ok`.
    #    La page croit avoir peint a 1450 (ancre a 1452) ; le vert n'arrive
    #    qu'a 1700.
    p = Pipeline(1000, 1700, 200, 30, captures, t_ancre=1452)
    echecs += _cas("peinture apres l'ancre -> latence_publication mesuree",
                   joue_nouvelle(p), lambda v: v.startswith("latence_publication tentatives=2"))

    if echecs:
        print(f"surface/declencheur : {echecs} cas en echec", file=sys.stderr)
        return 1
    print("SURFACE_DECLENCHEUR_OK")
    return 0


def lis_tentatives(chemin):
    tentatives = []
    try:
        with open(chemin, encoding="utf-8") as flux:
            for ligne in flux:
                morceaux = ligne.split()
                if len(morceaux) >= 2:
                    tentatives.append((int(morceaux[0]), morceaux[1]))
    except OSError:
        pass
    return tentatives


def main():
    if len(sys.argv) >= 2 and sys.argv[1] == "--test":
        return autotest()
    if len(sys.argv) < 3:
        print("usage: surface_declencheur.py <journal> <tentatives> [--fin]", file=sys.stderr)
        return 2
    try:
        with open(sys.argv[1], "rb") as flux:
            texte = flux.read().decode("utf-8", "replace")
    except OSError as exc:
        print(f"journal illisible : {exc}", file=sys.stderr)
        return 2
    d = decide(texte, lis_tentatives(sys.argv[2]), fin="--fin" in sys.argv[3:])
    verdict = d["verdict"].replace(" ", "_")
    # Prefixe `surf_` : le banc evalue cette ligne dans son propre shell, ou
    # `verdict` et `seq` ont deja un sens.
    print(f"surf_action={d['action']} surf_verdict={verdict or '-'} surf_seq={d['seq']}"
          f" surf_t_trame={d['t_trame']} surf_pompe={d['pompe']} surf_seq_ancre={d['seq_ancre']}"
          f" surf_voulue={d['voulue']} surf_tentative={d['tentative']} surf_decodees={d['decodees']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
