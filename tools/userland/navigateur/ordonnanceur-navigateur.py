"""La latence du chrome pendant que le vrai renderer travaille.

## Ce que cette sonde ajoute a `ordonnanceur-probe.c`

`ordonnanceur-probe` a etabli que les deux classes d'ordonnancement de Bouchaud
OS font ce qu'elles disent : sous huit processus de calcul, le pire retard d'un
reveil a 16 ms passe de huit millisecondes a une quand le processus qui se
reveille est declare interactif.

Mais ses processus de calcul etaient une boucle arithmetique. Rien ne disait que
le renderer d'un navigateur se comporte comme cette boucle : il alloue, il
touche beaucoup de memoire, il attend parfois une ressource, et il se fait
interrompre par ses propres messages. Il restait donc une inference — « le
renderer ressemble assez a une boucle de calcul pour que la mesure vaille ».

Cette sonde supprime l'inference. La charge est un **vrai renderer**, forke par
`superviseur.py`, qui met en page une vraie feuille lourde, dans le vrai
protocole. Ce qu'on mesure est ce que l'utilisateur ressent : le retard du
battement du chrome.

## Le dispositif

    Navigateur (ce processus)  : se reveille toutes les 16 ms, bat, recolte
    Renderer   (processus fils): remet en page et repeint sans repit

Deux passes, dans cet ordre, parce que la premiere sert de temoin a la seconde :

  1. **a egalite** — les deux processus en classe normale ;
  2. **differencie** — le navigateur interactif, le renderer normal.

Et une passe a vide avant les deux, pour connaitre le plancher de l'instrument :
un chrome qui bat seul mesure la resolution de l'horloge et rien d'autre.

## Ce que la sonde refuse de faire

Conclure quand elle ne mesure rien. Sur une machine a plusieurs cœurs peu
chargee, le renderer et le chrome tournent cote a cote et la priorite ne change
rien — ce n'est pas un echec de la priorite, c'est l'absence de contention. La
sonde le dit et n'affirme aucune amelioration. C'est la meme regle que
`ordonnanceur-probe.c`, et elle existe parce qu'une sonde qui valide n'importe
quelle implantation est pire qu'une sonde absente.

    BO_SCRIPT=ordonnanceur-navigateur.py ./test-moteur.sh        # hote de dev
    exec /bo-navigateur /usr/share/bo-navigateur/ordonnanceur-navigateur.py
"""

import json
import os
import statistics
import sys
import time
import types

# --- L'hote : le vrai s'il est la, un bouchon sinon --------------------------
#
# Copie deliberee de l'amorce de `test_moteur.py` plutot qu'un import : cette
# sonde doit pouvoir tourner sans que les 1600 verifications ne s'executent
# d'abord, et surtout sans qu'elles n'aient rempli l'espace d'adressage du
# parent avant le `fork` — ce qui fausserait la mesure autant que le budget.

try:
    import bo
    HOTE_REEL = True
except ImportError:
    def _largeur_texte(texte, taille, gras=False, fixe=False, famille=""):
        return len(texte) * taille * (0.62 if not fixe else 0.60) * (1.05 if gras else 1.0)

    def _rasterise(operations, largeur, hauteur):
        # Le bouchon ne peint rien : ce qui est mesure ici est la mise en page
        # et la cascade, pas la rasterisation, qui appartient a Qt.
        return bytes(largeur * hauteur * 4)

    bo = types.ModuleType("bo")
    bo.largeur_texte = _largeur_texte
    bo.hauteur_ligne = lambda taille, fixe=False: taille * 1.35
    bo.taille = lambda: (1280, 720)
    bo.titre = lambda _: None
    bo.redessiner = lambda: None
    bo.image = lambda octets: None
    bo.image_brute = lambda octets, l, h, i=-1: (0, l, h)
    bo.rasterise = _rasterise
    bo.police = lambda *a, **k: True
    bo.formats_images = lambda: ["png"]
    sys.modules["bo"] = bo
    HOTE_REEL = False

sys.path.insert(0, ".")

PERIODE_S = 0.016
REVEILS = 120
PAGE = "/page/feuille-lourde.html"

# En dessous, il n'y a pas de contention a corriger. Un quart de trame est le
# plus petit retard qu'un utilisateur puisse remarquer ; une machine ou le pire
# retard sous charge reste dessous n'apprend rien sur les classes
# d'ordonnancement — elle apprend seulement qu'elle avait un cœur de libre.
SEUIL_CONTENTION_MS = PERIODE_S * 1000.0 / 4.0


def _quantiles(valeurs):
    """Median, p95, p99 et pire, en millisecondes.

    Les quantiles hauts comptent plus que la mediane : une interface dont une
    trame sur vingt arrive en retard se voit, alors qu'une mediane parfaite ne
    se remarque pas.
    """
    ordonnees = sorted(valeurs)
    n = len(ordonnees)
    return {
        "median": ordonnees[n // 2] * 1000.0,
        "p95": ordonnees[min(n - 1, (n * 95) // 100)] * 1000.0,
        "p99": ordonnees[min(n - 1, (n * 99) // 100)] * 1000.0,
        "pire": ordonnees[-1] * 1000.0,
    }


def bat_et_mesure(renderer=None, reveils=REVEILS):
    """Se reveille a cadence fixe et note de combien on ne l'est pas.

    C'est exactement la boucle du chrome : `bo` appelle `tic` toutes les seize
    millisecondes, et `tic` bat la vue puis recolte ce qui vient du renderer.
    Un reveil demande a 16 ms qui arrive a 40 en accuse 24 de retard, et c'est
    ce retard-la que l'utilisateur ressent comme une saccade.
    """
    retards = []
    trames = 0
    attendu = time.monotonic()
    for _ in range(reveils):
        attendu += PERIODE_S
        reste = attendu - time.monotonic()
        if reste > 0:
            time.sleep(reste)
        debut = time.monotonic()
        retards.append(max(0.0, debut - attendu))
        if renderer is not None:
            try:
                renderer.bat()
                for nom, _charge in renderer.recolte(0.0):
                    if nom == "FRAME_READY":
                        trames += 1
            except Exception:  # noqa: BLE001 — un renderer perdu n'arrete pas la mesure
                pass
    return _quantiles(retards), trames


def _charge_lourde(renderer, base):
    """Met le renderer au travail, et attend qu'il ait commence pour de bon."""
    renderer.navigue(base + PAGE)
    vu, _ = renderer.attends("FRAME_READY", 120.0)
    return vu


def _pousse(renderer, tours=6):
    """Fait retravailler le renderer : chaque redimensionnement remet en page.

    Une page deja mise en page ne coute presque rien a repeindre ; ce qu'on veut
    mesurer est la contention, donc il faut que le renderer ait vraiment de quoi
    faire pendant que le chrome bat.
    """
    for tour in range(tours):
        try:
            renderer.redimensionne(900 + (tour % 3) * 40, 700)
        except Exception:  # noqa: BLE001
            return


def passe(libelle, base, interactif, journal):
    """Une passe complete : un renderer, une charge, une mesure."""
    from moteur import superviseur

    renderer = superviseur.Renderer(900, 700, journal=journal)
    try:
        if not _charge_lourde(renderer, base):
            return None
        # La priorite du **navigateur** est ce qu'on change ; celle du renderer
        # reste normale dans les deux passes. Le degrader donnerait « rien
        # d'autre ne tourne » plutot que « l'interface passe devant ».
        pose = True
        if interactif:
            try:
                os.setpriority(os.PRIO_PROCESS, 0, -5)
            except (OSError, PermissionError):
                pose = False
        _pousse(renderer)
        mesure, trames = bat_et_mesure(renderer)
        if interactif and pose:
            os.setpriority(os.PRIO_PROCESS, 0, 0)
        mesure["trames"] = trames
        mesure["priorite_posee"] = pose
        mesure["libelle"] = libelle
        return mesure
    finally:
        renderer.ferme()


def imprime(mesure):
    print("  %-26s median %6.2f ms  p95 %6.2f ms  p99 %6.2f ms  pire %7.2f ms"
          % (mesure["libelle"], mesure["median"], mesure["p95"],
             mesure["p99"], mesure["pire"]))


def principal():
    import serveur_test

    print("hote : %s" % ("Qt reel" if HOTE_REEL else "bouchon local"))
    serveur, base = serveur_test.demarre(0)
    journal = lambda niveau, texte: None  # noqa: E731
    try:
        print("\nlatence du battement du chrome (%d reveils a %d ms)"
              % (REVEILS, int(PERIODE_S * 1000)))
        vide, _ = bat_et_mesure(None)
        vide["libelle"] = "chrome seul (plancher)"
        imprime(vide)

        egalite = passe("renderer a egalite", base, False, journal)
        if egalite is None:
            print("  la page lourde n'a pas pu etre rendue — rien a conclure")
            return 2
        imprime(egalite)

        differencie = passe("chrome interactif", base, True, journal)
        if differencie is None:
            print("  seconde passe impossible — rien a conclure")
            return 2
        imprime(differencie)

        print()
        if not differencie["priorite_posee"]:
            print("  la priorite n'a pas pu etre posee (droits) — "
                  "aucune conclusion")
            verdict = "indisponible"
        elif egalite["pire"] < SEUIL_CONTENTION_MS:
            # La charge n'a pas degrade la latence de facon visible : plusieurs
            # cœurs libres, et le renderer n'a jamais eu a disputer le sien au
            # chrome. Il n'y a rien a ameliorer, donc rien a conclure sur
            # l'amelioration.
            #
            # Le seuil est en **millisecondes absolues**, pas en multiple du
            # plancher, et cela a coute une conclusion fausse : compare au
            # plancher, un pire retard de 0,52 ms contre 0,33 ms a vide passait
            # pour de la contention, et la passe suivante mesurait a 0,71 ms —
            # d'ou un « 0,7x », c'est-a-dire une degradation annoncee, tiree de
            # trois dixiemes de milliseconde de bruit. Un quart de trame est le
            # plus petit retard qu'un utilisateur puisse remarquer ; en dessous,
            # il n'y a rien a mesurer, quel que soit le rapport.
            print("  la charge ne degrade pas la latence sur cette machine "
                  "(%.2f ms au pire, seuil %.2f ms) — le renderer n'a pas eu a "
                  "disputer son cœur au chrome, la priorite n'a rien a corriger "
                  "ici, et la sonde n'affirme rien."
                  % (egalite["pire"], SEUIL_CONTENTION_MS))
            verdict = "sans contention"
        else:
            gain = egalite["pire"] / max(0.001, differencie["pire"])
            print("  pire retard : %.2f ms a egalite, %.2f ms avec priorite "
                  "(%.1fx)" % (egalite["pire"], differencie["pire"], gain))
            verdict = "mesure"

        rapport = {"hote": "qt" if HOTE_REEL else "bouchon",
                   "verdict": verdict, "plancher": vide,
                   "egalite": egalite, "differencie": differencie}
        chemin = os.environ.get("BO_RAPPORT_ORDONNANCEUR")
        if chemin:
            with open(chemin, "w", encoding="utf-8") as fichier:
                json.dump(rapport, fichier, ensure_ascii=False, indent=2,
                          sort_keys=True)
                fichier.write("\n")
        return 0
    finally:
        serveur_test.arrete(serveur)


if __name__ == "__main__":
    sys.exit(principal())
