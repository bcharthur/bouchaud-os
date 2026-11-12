#!/usr/bin/env python3
"""BOUCHAUD_INSTALL_SSD_V1 -- poser Bouchaud OS (disque A/B) sur un SSD DEDIE, depuis un hote Linux.

    installe_ssd.py liste
    installe_ssd.py installe --image DIR/disque-ab.img --cible /dev/sdX \\
                             --identite 'MODELE|SERIE|OCTETS' [--execute]

Ce que l'outil ne fait JAMAIS :
  - choisir un disque : `liste` montre, `installe` exige la cible ET son
    identite exacte (modele, numero de serie, taille en octets, lus dans
    /sys/block) -- un /dev/sdX qui a change de disque entre deux branchements
    est refuse ;
  - ecrire sans --execute : par defaut il affiche le plan (dry-run) ;
  - ecrire sans la phrase de confirmation, TAPEE sur l'entree standard :
    `EFFACER <SERIE>` ;
  - ecrire sur une partition, un disque dont une partition est montee ou sert
    de swap, ou le disque qui porte / , /boot, /home ;
  - ecrire une image dont l'empreinte differe de disque-ab.img.sha256.

Apres ecriture : fsync, RELECTURE integrale comparee a l'empreinte, puis la
derniere partition (BOUCHAUD-STATE) est etendue jusqu'a la fin du disque
(GPT primaire et secours reecrites, CRC recalcules, relues). Les slots, META
et RECOVERY ne bougent pas.

Codes : 0 succes, 2 refus (`INSTALL_REFUSEE raison=...`), 1 echec d'E/S.
Mode test (tests uniquement) : BO_INSTALL_RACINE_TEST=DIR remplace /sys,
/proc/mounts, /proc/swaps par DIR/... et accepte un FICHIER comme cible ;
il refuse toute cible sous /dev.
"""
from __future__ import annotations

import argparse
import hashlib
import os
import stat
import struct
import sys
import uuid
import zlib
from pathlib import Path

SECTEUR = 512
MIB = 1 << 20
TYPE_STATE = uuid.UUID("B0DC8A5D-0001-4B0D-9E21-424F55434844").bytes_le
POINTS_SYSTEME = {"/", "/boot", "/boot/efi", "/home", "/usr", "/var"}


class Refus(Exception):
    pass


# ---------------------------------------------------------------- environnement

def racine_test() -> Path | None:
    r = os.environ.get("BO_INSTALL_RACINE_TEST")
    return Path(r) if r else None


def chemin_sys(rel: str) -> Path:
    r = racine_test()
    return (r / rel.lstrip("/")) if r else Path(rel)


def lit(p: Path) -> str:
    try:
        return p.read_text(errors="replace").strip()
    except OSError:
        return ""


def disques() -> list[dict]:
    base = chemin_sys("/sys/block")
    r = []
    for d in sorted(base.iterdir()) if base.is_dir() else []:
        nom = d.name
        if nom.startswith(("loop", "ram", "zram", "dm-", "md", "sr")):
            continue
        r.append({
            "nom": nom,
            "modele": lit(d / "device/model") or "?",
            "serie": lit(d / "device/serial") or lit(d / "serial") or "?",
            "octets": int(lit(d / "size") or 0) * SECTEUR,
            "amovible": lit(d / "removable") == "1",
            "partitions": sorted(p.name for p in d.iterdir() if p.name.startswith(nom) and (p / "partition").exists()),
        })
    return r


def montages() -> list[tuple[str, str]]:
    r = []
    for l in lit(chemin_sys("/proc/mounts")).splitlines():
        c = l.split()
        if len(c) >= 2:
            r.append((c[0], c[1]))
    return r


def swaps() -> list[str]:
    return [l.split()[0] for l in lit(chemin_sys("/proc/swaps")).splitlines()[1:] if l.split()]


# ---------------------------------------------------------------------- refus

def disque_de(cible: str) -> dict:
    test = racine_test()
    if test:
        if cible.startswith("/dev"):
            raise Refus("mode_test_sur_un_vrai_peripherique")
        if not Path(cible).is_file():
            raise Refus("cible_absente")
        nom = Path(cible).name
    else:
        try:
            st = os.stat(cible)
        except OSError:
            raise Refus("cible_absente")
        if not stat.S_ISBLK(st.st_mode):
            raise Refus("cible_pas_un_peripherique_bloc")
        nom = Path(os.path.realpath(cible)).name
    for d in disques():
        if d["nom"] == nom:
            return d
    if chemin_sys(f"/sys/class/block/{nom}/partition").exists():
        raise Refus(f"cible_est_une_partition ({nom}) : donner le disque entier")
    raise Refus(f"disque_inconnu ({nom})")


def verifie_cible(d: dict, identite: str) -> None:
    attendu = f"{d['modele']}|{d['serie']}|{d['octets']}"
    if identite != attendu:
        raise Refus(f"identite_differente (lue : {attendu!r}, donnee : {identite!r})")
    if d["serie"] == "?":
        raise Refus("numero_de_serie_illisible (identite non etablie)")
    noms = {f"/dev/{d['nom']}", *(f"/dev/{p}" for p in d["partitions"])}
    for dev, point in montages():
        if dev in noms:
            raise Refus(f"partition_montee {dev} sur {point}" + (" (disque systeme)" if point in POINTS_SYSTEME else ""))
    for s in swaps():
        if s in noms:
            raise Refus(f"swap_actif {s}")


def empreinte(p: Path, octets: int | None = None) -> str:
    h = hashlib.sha256()
    reste = octets if octets is not None else p.stat().st_size
    with p.open("rb") as f:
        while reste > 0:
            b = f.read(min(reste, 4 * MIB))
            if not b:
                raise Refus("relecture_courte")
            h.update(b)
            reste -= len(b)
    return h.hexdigest()


def verifie_image(image: Path) -> str:
    somme = image.with_name(image.name + ".sha256")
    if not image.is_file() or not somme.is_file():
        raise Refus("image_ou_empreinte_absente")
    attendu = lit(somme).split()[0]
    if empreinte(image) != attendu:
        raise Refus("empreinte_image_differente")
    return attendu


# ----------------------------------------------------------------------- GPT

def _entete(b: bytes) -> dict:
    if b[:8] != b"EFI PART":
        raise Refus("gpt_absente")
    (taille,) = struct.unpack_from("<I", b, 12)
    h = bytearray(b[:taille])
    crc = struct.unpack_from("<I", h, 16)[0]
    struct.pack_into("<I", h, 16, 0)
    if zlib.crc32(h) & 0xFFFFFFFF != crc:
        raise Refus("gpt_crc_entete")
    cur, sec, prem, dern = struct.unpack_from("<QQQQ", b, 24)
    lba_ent, n, t, crc_ent = struct.unpack_from("<QIII", b, 72)
    return {"taille": taille, "brut": bytes(b[:taille]), "courant": cur, "secours": sec, "premier": prem,
            "dernier": dern, "lba_entrees": lba_ent, "n": n, "t": t, "crc_entrees": crc_ent}


def _encode_entete(e: dict, courant: int, secours: int, dernier: int, lba_ent: int, crc_ent: int) -> bytes:
    h = bytearray(e["brut"])
    struct.pack_into("<QQ", h, 24, courant, secours)
    struct.pack_into("<Q", h, 48, dernier)
    struct.pack_into("<Q", h, 72, lba_ent)
    struct.pack_into("<I", h, 88, crc_ent)
    struct.pack_into("<I", h, 16, 0)
    struct.pack_into("<I", h, 16, zlib.crc32(h) & 0xFFFFFFFF)
    return bytes(h) + b"\0" * (SECTEUR - len(h))


def lit_gpt(f) -> tuple[dict, bytearray]:
    f.seek(SECTEUR)
    e = _entete(f.read(SECTEUR))
    f.seek(e["lba_entrees"] * SECTEUR)
    ent = bytearray(f.read(e["n"] * e["t"]))
    if zlib.crc32(ent) & 0xFFFFFFFF != e["crc_entrees"]:
        raise Refus("gpt_crc_entrees")
    return e, ent


def partitions(e: dict, ent: bytes) -> list[tuple[int, bytes, int, int]]:
    r = []
    for i in range(e["n"]):
        o = i * e["t"]
        if ent[o:o + 16] != b"\0" * 16:
            prem, dern = struct.unpack_from("<QQ", ent, o + 32)
            r.append((i, bytes(ent[o:o + 16]), prem, dern))
    return r


def etend_state(f, octets_disque: int) -> tuple[int, int]:
    """STATE (derniere partition) jusqu'a la fin du disque. Rend (avant, apres) en Mio."""
    e, ent = lit_gpt(f)
    parts = partitions(e, ent)
    i, ty, prem, dern = max(parts, key=lambda p: p[3])
    if ty != TYPE_STATE:
        raise Refus("derniere_partition_n_est_pas_state")
    secteurs = octets_disque // SECTEUR
    sect_ent = (e["n"] * e["t"] + SECTEUR - 1) // SECTEUR
    secours = secteurs - 1
    dernier_utilisable = secours - sect_ent - 1
    nouveau = ((dernier_utilisable + 1) // (MIB // SECTEUR)) * (MIB // SECTEUR) - 1
    if nouveau < dern:
        raise Refus("disque_plus_petit_que_l_image")
    struct.pack_into("<Q", ent, i * e["t"] + 40, nouveau)
    crc_ent = zlib.crc32(ent) & 0xFFFFFFFF
    lba_ent_secours = secours - sect_ent
    # Secours d'abord (entrees puis en-tete), primaire ensuite : une coupure
    # entre les deux laisse un primaire VALIDE (l'ancien).
    f.seek(lba_ent_secours * SECTEUR); f.write(ent)
    f.seek(secours * SECTEUR); f.write(_encode_entete(e, secours, 1, dernier_utilisable, lba_ent_secours, crc_ent))
    f.flush(); os.fsync(f.fileno())
    f.seek(e["lba_entrees"] * SECTEUR); f.write(ent)
    f.seek(SECTEUR); f.write(_encode_entete(e, 1, secours, dernier_utilisable, e["lba_entrees"], crc_ent))
    # MBR protecteur : taille du disque.
    f.seek(446 + 12); f.write(struct.pack("<I", min(secteurs - 1, 0xFFFFFFFF)))
    f.flush(); os.fsync(f.fileno())
    # Relecture : les deux en-tetes et leurs entrees doivent valider.
    e2, ent2 = lit_gpt(f)
    f.seek(secours * SECTEUR)
    s2 = _entete(f.read(SECTEUR))
    f.seek(s2["lba_entrees"] * SECTEUR)
    if zlib.crc32(f.read(s2["n"] * s2["t"])) & 0xFFFFFFFF != s2["crc_entrees"] or ent2 != ent:
        raise Refus("gpt_relue_differente")
    return (dern - prem + 1) * SECTEUR // MIB, (nouveau - prem + 1) * SECTEUR // MIB


# ---------------------------------------------------------------- commandes

def liste() -> int:
    for d in disques():
        print(f"/dev/{d['nom']:<10} {d['octets'] / 1e9:8.1f} Go  amovible={int(d['amovible'])}  "
              f"identite='{d['modele']}|{d['serie']}|{d['octets']}'")
    print("Aucun disque n'est choisi automatiquement : copier l'identite du SSD DEDIE dans --identite.")
    return 0


def installe(a: argparse.Namespace) -> int:
    image = Path(a.image)
    d = disque_de(a.cible)
    verifie_cible(d, a.identite)
    h = verifie_image(image)
    taille = image.stat().st_size
    # La GPT de l'IMAGE est valide et finit par STATE : verifie AVANT d'ecrire.
    with image.open("rb") as f:
        e, ent = lit_gpt(f)
        if max(partitions(e, ent), key=lambda p: p[3])[1] != TYPE_STATE:
            raise Refus("derniere_partition_n_est_pas_state")
    if taille > d["octets"]:
        raise Refus("image_plus_grande_que_le_disque")
    print(f"PLAN cible=/dev/{d['nom']} identite='{a.identite}' image={image.name} sha256={h[:16]} "
          f"octets={taille} disque={d['octets']} : TOUT LE CONTENU DU DISQUE SERA DETRUIT")
    if not a.execute:
        print("INSTALL_DRY_RUN rien n'a ete ecrit (ajouter --execute)")
        return 0
    phrase = f"EFFACER {d['serie']}"
    print(f"Taper exactement : {phrase}")
    if sys.stdin.readline().rstrip("\n") != phrase:
        raise Refus("confirmation_absente_ou_fausse")
    # Rien n'a change entre la verification et l'ecriture ?
    verifie_cible(disque_de(a.cible), a.identite)
    with open(a.cible, "r+b", buffering=0) as f, image.open("rb") as src:
        while True:
            b = src.read(4 * MIB)
            if not b:
                break
            f.write(b)
        os.fsync(f.fileno())
    if empreinte(Path(a.cible), taille) != h:
        print("INSTALL_ECHEC relecture_differente", file=sys.stderr)
        return 1
    with open(a.cible, "r+b", buffering=0) as f:
        avant, apres = etend_state(f, d["octets"])
    print(f"INSTALL_OK cible=/dev/{d['nom']} sha256={h[:16]} relu=ok state_mio={avant}->{apres} PHYSICAL=NOT_RUN")
    return 0


def main(argv: list[str]) -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sp = p.add_subparsers(dest="cmd", required=True)
    sp.add_parser("liste")
    i = sp.add_parser("installe")
    i.add_argument("--image", required=True)
    i.add_argument("--cible", required=True)
    i.add_argument("--identite", required=True)
    i.add_argument("--execute", action="store_true")
    a = p.parse_args(argv[1:])
    try:
        return liste() if a.cmd == "liste" else installe(a)
    except Refus as e:
        print(f"INSTALL_REFUSEE raison={e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
