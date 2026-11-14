#!/usr/bin/env python3
"""BOUCHAUD_E2E_LADYBIRD_V1 -- disque des parcours E2E.

    prepare_disque.py --artefact DIR --certs FICHIER --url URL --sortie DISQUE.img
    prepare_disque.py --image ladybird-browser.img --url URL --sortie DISQUE.img

Deux sources, UN resultat : une archive USTAR (alignee sur 512 octets, comme
tools/userland/mkdisk.sh) dont l'autorun est celui d'une SESSION NORMALE --
le bureau Atelier, le navigateur ouvert AU MENU par le banc, comme par un
humain (aucun `BO_AUTOSTART_BROWSER`) -- et dont la page d'accueil est le site
de reference du banc (URL).

  --artefact : l'artefact Ladybird de la CI (binaires, ressources), plus le
               magasin d'autorites (certificats) ; autorun de run.ps1 en mode
               interactif (M9, M11, BrowserHost, fuseau, surface simple,
               journal de production).
  --image    : le disque fabrique par run.ps1 sur Windows ; seule la ligne de
               l'URL d'accueil change, l'autostart eventuel est retire. La
               SOURCE n'est jamais ouverte en ecriture.

Portable (Python seul) : meme commande sous Linux et sous Windows.
"""
import argparse
import io
import re
import sys
import tarfile
from pathlib import Path

BINAIRES = ["BouchaudBrowserHost", "WebContent", "RequestServer", "ImageDecoder", "WebWorker", "Compositor", "WebDriver"]


def autorun_session(url: str) -> str:
    return "\n".join([
        "uname", "df",
        'echo "=== Bouchaud OS E2E : bureau Atelier, navigateur au menu ==="',
        "export BOUCHAUD_M9=1",
        f"export BOUCHAUD_M9_URL='{url}'",
        "export BOUCHAUD_M11=1",
        "export BOUCHAUD_BROWSER_HOST=1",
        "export BOUCHAUD_TIME_ZONE=Europe/Paris",
        "export BOUCHAUD_ALLOW_POPUPS=1",
        "gui-surface simple",
        "profil-rip off",
        "desktop", ""])


MAGASIN = ("./etc/ssl/certs/ca-certificates.crt", "etc/ssl/certs/ca-certificates.crt")
CA_DE_TEST: bytes | None = None  # --ca-test : autorite generee par tls/genere_tls.py


def avec_ca_de_test(magasin: bytes, ca: bytes | None = None) -> bytes:
    """Le magasin d'autorites + l'autorite de TEST du banc, si fournie.

    Seul le disque E2E la recoit : E2E-03 prouve qu'un certificat EXPIRE
    signe par une autorite de confiance est refuse pour son expiration.
    """
    ca = CA_DE_TEST if ca is None else ca
    if not ca:
        return magasin
    return magasin.rstrip(b"\n") + b"\n" + ca


def ajoute(tar: tarfile.TarFile, nom: str, donnees: bytes, mode: int) -> None:
    info = tarfile.TarInfo(nom)
    info.size, info.mode, info.uid, info.gid, info.uname, info.gname = len(donnees), mode, 0, 0, "root", "root"
    tar.addfile(info, io.BytesIO(donnees))


def dossier(tar: tarfile.TarFile, nom: str) -> None:
    info = tarfile.TarInfo(nom)
    info.type, info.mode, info.uname, info.gname = tarfile.DIRTYPE, 0o755, "root", "root"
    tar.addfile(info)


def depuis_artefact(artefact: Path, certs: Path, url: str, tar: tarfile.TarFile) -> None:
    for b in BINAIRES:
        if not (artefact / b).is_file():
            raise SystemExit(f"E2E_DISQUE_REFUS binaire absent de l'artefact : {b}")
    for d in ["./usr", "./usr/libexec", "./usr/libexec/ladybird", "./usr/share", "./usr/share/ladybird",
              "./etc", "./etc/ssl", "./etc/ssl/certs"]:
        dossier(tar, d)
    for b in BINAIRES:
        ajoute(tar, f"./usr/libexec/ladybird/{b}", (artefact / b).read_bytes(), 0o755)
    ajoute(tar, "./bo-navigateur", (artefact / "BouchaudBrowserHost").read_bytes(), 0o755)
    ressources = artefact / "resources"
    if ressources.is_dir():
        for p in sorted(ressources.rglob("*")):
            rel = p.relative_to(ressources).as_posix()
            if p.is_dir():
                dossier(tar, f"./usr/share/ladybird/{rel}")
            else:
                ajoute(tar, f"./usr/share/ladybird/{rel}", p.read_bytes(), 0o644)
    ajoute(tar, "./etc/ssl/certs/ca-certificates.crt", avec_ca_de_test(certs.read_bytes()), 0o644)
    ajoute(tar, "./autorun", autorun_session(url).encode(), 0o644)


def depuis_image(image: Path, url: str, tar: tarfile.TarFile) -> None:
    with tarfile.open(image, mode="r:") as src:
        vu = False
        for m in src.getmembers():
            if m.name in ("./autorun", "autorun") and m.isfile():
                lignes = [l for l in src.extractfile(m).read().decode("utf-8").split("\n")
                          if l.strip() != "export BO_AUTOSTART_BROWSER=1"]
                lignes = [l for l in lignes if not l.startswith("export BOUCHAUD_M9_URL=")]
                if "desktop" not in lignes:
                    raise SystemExit("E2E_DISQUE_REFUS autorun sans ligne desktop")
                i = lignes.index("desktop")
                lignes.insert(i, f"export BOUCHAUD_M9_URL='{url}'")
                ajoute(tar, m.name, "\n".join(lignes).encode(), m.mode)
                vu = True
            elif m.name in MAGASIN and m.isfile():
                ajoute(tar, m.name, avec_ca_de_test(src.extractfile(m).read()), m.mode)
            elif m.isfile():
                tar.addfile(m, src.extractfile(m))
            else:
                tar.addfile(m)
        if not vu:
            raise SystemExit("E2E_DISQUE_REFUS autorun absent de l'image")


# Zone persistante, comme tools/userland/mkdisk.sh (src/fs/persistance) :
# 128 Mio a la FIN de l'image. Sans elle, le noyau declare « disque trop
# petit, zone absente », /persist n'existe pas, et le BrowserHost s'arrete
# aussitot avec le code 1 (parcours KVM a 5127a0e et a4432d1 : quatre
# lancements, quatre sorties, relances epuisees).
ZONE_SECTEURS = 262144
SECTEUR_CONTENU = 1025
PLANCHER_ARCHIVE = (SECTEUR_CONTENU + 1) * 512


def ajoute_zone_persistante(image: Path) -> None:
    taille = image.stat().st_size
    archive = taille + (-taille) % 512
    archive = max(archive, PLANCHER_ARCHIVE)
    with image.open("ab") as f:
        f.write(b"\0" * (archive - taille))
        bloc = b"\0" * (1024 * 1024)
        for _ in range(ZONE_SECTEURS * 512 // len(bloc)):
            f.write(bloc)


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--artefact", type=Path)
    p.add_argument("--certs", type=Path)
    p.add_argument("--ca-test", dest="ca_test", type=Path, help="tls/genere_tls.py DOSSIER -> DOSSIER/ca.pem")
    p.add_argument("--image", type=Path)
    p.add_argument("--url", required=True)
    p.add_argument("--sortie", required=True, type=Path)
    o = p.parse_args(argv[1:])
    if bool(o.artefact) == bool(o.image):
        p.error("une source : --artefact (avec --certs) OU --image")
    if not re.fullmatch(r"https?://[A-Za-z0-9.:/_-]+", o.url):
        p.error("URL refusee (caracteres hors de l'autorun)")
    if o.image and o.image.resolve() == o.sortie.resolve():
        p.error("la sortie ne peut pas etre la source")
    global CA_DE_TEST
    if o.ca_test:
        CA_DE_TEST = o.ca_test.read_bytes()
    tmp = o.sortie.with_name(o.sortie.name + ".tmp")
    with tarfile.open(tmp, mode="w", format=tarfile.USTAR_FORMAT) as tar:
        if o.artefact:
            if not o.certs or not o.certs.is_file():
                p.error("--certs requis avec --artefact")
            depuis_artefact(o.artefact, o.certs, o.url, tar)
        else:
            depuis_image(o.image, o.url, tar)
    ajoute_zone_persistante(tmp)
    tmp.replace(o.sortie)
    print(f"E2E_DISQUE_OK sortie={o.sortie} octets={o.sortie.stat().st_size} url={o.url}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
