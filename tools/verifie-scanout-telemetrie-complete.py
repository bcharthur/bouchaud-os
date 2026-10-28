#!/usr/bin/env python3
"""Le banc scanout virtio-gpu ne juge qu'une telemetrie entierement ecrite
(P10_TELEMETRIE_COMPLETE_V1).

Run 40715886448 (77665751), scanout direct sous KVM : le banc arretait QEMU
des que le DEBUT de `[GUI-VIRTIO-GPU]` apparaissait ; la ligne jugee etait
`... us_par_present=250 ac`, et quatre controles des compteurs ont echoue
sur un fragment.

Regles :

  1. le motif `TELEMETRIE` nomme le dernier champ emis
     (`regions_differees=[0-9]+$`) ;
  2. l'attente `FINAL` du demarrage virtio est ce motif, verifiee par
     `ligne_complete.py` (lignes TERMINEES seulement), jamais par `grep` ;
  3. la mesure jugee est la derniere ligne entiere (`--derniere`) ;
  4. sans ligne entiere : un refus explicite, et les controles des compteurs
     ne sont pas evalues ;
  5. `ligne_complete.py` ecarte le fragment qui suit le dernier saut de ligne,
     et le temoin negatif (`test_ligne_complete.py`) rejoue le fragment du
     run 40715886448.

Fail-closed ; tests negatifs par mutation.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
F = {
    "banc": RACINE / "tools/ci/run_virtio_gpu_scanout.sh",
    "outil": RACINE / "tools/ci/ligne_complete.py",
    "test": RACINE / "tools/ci/reliability/test_ligne_complete.py",
}
MOTIF = "TELEMETRIE='\\[GUI-VIRTIO-GPU\\] presents_ok=.* regions_differees=[0-9]+$'"
ATTENTE = '        python3 tools/ci/ligne_complete.py "$log" "$FINAL" && break'
LANCE = 'SECONDE=3 FINAL="$TELEMETRIE" demarre virtio'
MESURE = 'MESURE=$(python3 tools/ci/ligne_complete.py "$SORTIE/serie-virtio.log" "$TELEMETRIE" --derniere'
REFUS = 'exige "telemetrie [GUI-VIRTIO-GPU] entierement ecrite (sinon les compteurs ne sont pas juges)" test -n "$MESURE"'
GARDE = 'if [ -z "$MESURE" ]; then'
COMPTEURS = 'exige "presentations virtio : aucune en echec, au moins une reussie"'
FRAGMENT = "    for brut in morceaux[:-1]:"


def verifie(t: dict) -> list:
    fautes = []
    b = t["banc"]
    if MOTIF not in b:
        fautes.append("banc : le motif de telemetrie ne nomme plus le dernier champ emis.")
    if ATTENTE not in b or 'grep -aqE "$FINAL"' in b:
        fautes.append("banc : l'attente FINAL ne verifie plus une ligne ENTIERE.")
    if LANCE not in b:
        fautes.append("banc : le demarrage virtio n'attend plus la telemetrie entiere.")
    if MESURE not in b or "grep -a '\\[GUI-VIRTIO-GPU\\]' | tail -1" in b:
        fautes.append("banc : la mesure jugee n'est plus la derniere ligne entiere.")
    ordre = [b.find(j) for j in (MESURE, REFUS, GARDE, COMPTEURS)]
    if min(ordre) < 0 or ordre != sorted(ordre):
        fautes.append("banc : sans ligne entiere, le refus explicite ne precede plus les controles des compteurs.")
    if FRAGMENT not in t["outil"]:
        fautes.append("ligne_complete.py : le fragment final (sans saut de ligne) n'est plus ecarte.")
    if "def test_le_fragment_du_run_40715886448_est_refuse(" not in t["test"]:
        fautes.append("temoin negatif absent : test_le_fragment_du_run_40715886448_est_refuse.")
    return fautes


def negatifs(t: dict) -> tuple:
    mutations = [
        ("motif sans dernier champ", "banc", " regions_differees=[0-9]+$'", "'"),
        ("attente par grep", "banc", ATTENTE, '        grep -aqE "$FINAL" "$log" && break'),
        ("attente du seul debut", "banc", LANCE, "SECONDE=3 FINAL='\\[GUI-VIRTIO-GPU\\]' demarre virtio"),
        ("mesure sur un fragment", "banc", MESURE + " \\\n  | sed",
         "MESURE=$(propre \"$SORTIE/serie-virtio.log\" | grep -a '\\[GUI-VIRTIO-GPU\\]' | tail -1 \\\n  | sed"),
        ("refus retire", "banc", REFUS + "\n", ""),
        ("fragment final retenu", "outil", FRAGMENT, "    for brut in morceaux:"),
        ("temoin retire", "test", "def test_le_fragment_du_run_40715886448_est_refuse(", "def test_x("),
    ]
    echecs = []
    for nom, cle, avant, apres in mutations:
        if avant not in t[cle]:
            echecs.append(f"mutation inapplicable : {nom}")
            continue
        m = dict(t)
        m[cle] = t[cle].replace(avant, apres, 1)
        if not verifie(m):
            echecs.append(f"mutation non detectee : {nom}")
    return echecs, len(mutations)


def main() -> int:
    t = {k: p.read_text(encoding="utf-8") for k, p in F.items()}
    echecs, n = negatifs(t)
    fautes = verifie(t) + echecs
    if fautes:
        for f in fautes:
            print(f"verifie-scanout-telemetrie-complete: {f}")
        return 1
    print(f"verifie-scanout-telemetrie-complete: OK (attente et mesure sur une ligne entiere, refus explicite, "
          f"temoin du fragment ; {n} negatifs)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
