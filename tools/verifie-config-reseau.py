#!/usr/bin/env python3
"""La configuration d'eth0 se publie et se lit par generation, jamais champ a champ.

BOUCHAUD_NET_CONFIG_GENERATION_V1

# Le defaut

Adresse, passerelle, resolveur, masque et nom du reseau etaient des
`static mut` ecrits un a un, en deux appels (`set_config` puis
`pose_identite_reseau`), et lus un a un. Un lecteur pouvait combiner l'adresse
d'un bail avec le masque ou la passerelle du precedent ; deux ecrivains
faisaient des lecture-modification-ecriture sans exclusion. Le gros verrou
n'a jamais protege ces champs : le fil DHCP ne le prenait pas.

# La regle

  * aucun `static mut` OUR_IP, GW_IP, DNS_IP, MASQUE, NOM_RESEAU(_LEN),
    GW_MAC ni DEMARRAGE dans src/net ;
  * src/net/config.rs : la lecture relit la sequence apres une barriere
    Acquire ; l'ecriture passe par `modifie`, sous le verrou `ECRIVAINS`
    (propre a la configuration), sequence impaire / barriere Release /
    champs / sequence paire Release ;
  * le client DHCP publie un bail par UN appel (`applique_bail`) ;
  * aucune fonction de src/ ne combine deux lectures champ a champ
    (`our_ip()`, `gateway()`, `dns_server()`) : qui en veut deux prend un
    instantane ;
  * la boite noire lit par `instantane_borne` (contexte de faute).

Fail-closed ; six tests negatifs.

Le banc `netcfg-banc` eprouve le meme `Publication` sur sa propre instance :
il ne doit jamais ecrire la configuration reelle.
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
CONFIG = "src/net/config.rs"
DHCP = "src/net/application/dhcp.rs"
BOITE = "src/kernel/debug/blackbox.rs"

ANCIENS = re.compile(
    r"\bstatic\s+mut\s+(OUR_IP|GW_IP|DNS_IP|MASQUE|NOM_RESEAU|NOM_RESEAU_LEN|GW_MAC|DEMARRAGE)\b")
CHAMPS = re.compile(r"\b(our_ip|gateway|dns_server)\(\)")


def sans_commentaires(texte: str) -> str:
    return "\n".join(l.split("//", 1)[0] for l in texte.splitlines())


def corps_de_fonctions(texte: str):
    """(nom, corps) de chaque `fn` du texte, par appariement d'accolades."""
    for m in re.finditer(r"\bfn\s+(\w+)\s*(?:<[^>{]*>)?\s*\(", texte):
        ouverture = texte.find("{", m.end())
        point_virgule = texte.find(";", m.end())
        if ouverture < 0 or (0 <= point_virgule < ouverture):
            continue
        profondeur = 0
        for i in range(ouverture, len(texte)):
            if texte[i] == "{":
                profondeur += 1
            elif texte[i] == "}":
                profondeur -= 1
                if profondeur == 0:
                    yield m.group(1), texte[ouverture:i + 1]
                    break


def verifie(racine: Path) -> list[str]:
    fautes = []
    src = racine / "src"
    if not src.is_dir():
        return ["src/ introuvable"]
    for chemin in sorted(src.rglob("*.rs")):
        relatif = chemin.relative_to(racine).as_posix()
        texte = sans_commentaires(chemin.read_text(encoding="utf-8", errors="replace"))
        if relatif.startswith("src/net/"):
            for m in ANCIENS.finditer(texte):
                fautes.append(f"{relatif} : `static mut {m.group(1)}` est revenu")
        for nom, corps in corps_de_fonctions(texte):
            lus = set(CHAMPS.findall(corps))
            if len(lus) > 1:
                fautes.append(f"{relatif} : fn {nom} combine {sorted(lus)} champ a champ"
                              " -- prendre config::instantane()")
    try:
        config = sans_commentaires((racine / CONFIG).read_text(encoding="utf-8"))
        dhcp = sans_commentaires((racine / DHCP).read_text(encoding="utf-8"))
        boite = sans_commentaires((racine / BOITE).read_text(encoding="utf-8"))
    except OSError as e:
        return fautes + [f"lecture impossible : {e}"]
    lecture = next((c for n, c in corps_de_fonctions(config) if n == "essaie_une_lecture"), "")
    if not re.search(r"fence\(Ordering::Acquire\);\s*\(self\.sequence\.load\(", lecture):
        fautes.append(f"{CONFIG} : la lecture ne relit plus la sequence apres une barriere Acquire")
    if "& 1 == 1" not in lecture:
        fautes.append(f"{CONFIG} : la lecture accepte une sequence impaire (ecriture en cours)")
    ecriture = next((c for n, c in corps_de_fonctions(config) if n == "modifie"), "")
    attendu = ["self.ecrivains.lock()", "s + 1", "fence(Ordering::Release)", "ecrit_champs", "s + 2, Ordering::Release"]
    positions = [ecriture.find(a) for a in attendu]
    if -1 in positions or positions != sorted(positions):
        fautes.append(f"{CONFIG} : `modifie` ne suit plus verrou / impair / Release / champs / pair")
    if not re.search(r"\becrivains\s*:\s*SpinLockIrq<", config):
        fautes.append(f"{CONFIG} : les ecrivains ne sont plus sous leur SpinLockIrq propre")
    if not re.search(r"static\s+ETH0\s*:\s*Publication\b", config):
        fautes.append(f"{CONFIG} : la configuration d'eth0 n'est plus une `Publication`")
    banc = next((c for n, c in corps_de_fonctions(config) if n == "fil_banc"), "")
    if "ETH0" in banc or re.search(r"(?<![.\w])modifie\(", banc):
        fautes.append(f"{CONFIG} : le banc ecrit de nouveau la configuration REELLE")
    if "applique_bail(" not in dhcp or "set_config(" in dhcp or "pose_identite_reseau(" in dhcp:
        fautes.append(f"{DHCP} : un bail n'est plus publie en UN appel (applique_bail)")
    if "instantane_borne(" not in boite:
        fautes.append(f"{BOITE} : la boite noire ne lit plus la configuration de facon bornee")
    return fautes


def mutation(fichier: str, avant: str, apres: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        copie = Path(tmp)
        for chemin in (RACINE / "src").rglob("*.rs"):
            dest = copie / chemin.relative_to(RACINE)
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text(chemin.read_text(encoding="utf-8", errors="replace"), encoding="utf-8")
        cible = copie / fichier
        texte = cible.read_text(encoding="utf-8")
        if avant not in texte:
            return False
        cible.write_text(texte.replace(avant, apres, 1), encoding="utf-8")
        return bool(verifie(copie))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("configuration reseau : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        ("src/net/mod.rs", "pub mod config;", "pub mod config;\nstatic mut MASQUE: [u8; 4] = [0; 4];"),
        (CONFIG, "        fence(Ordering::Acquire);\n        (self.sequence.load(Ordering::Relaxed) == avant).then_some(c)",
         "        Some(c)"),
        (CONFIG, "        let _seul = self.ecrivains.lock();\n", ""),
        (CONFIG, "            BANC.modifie(|c| *c = g);", "            modifie(|c| *c = g);"),
        (DHCP, "let publiee = net::applique_bail(", "net::set_config(ack.your_ip, ack.router, ack.dns);\n    let publiee = net::applique_bail("),
        ("src/net/transport/smol_tcp.rs", "    let gw = config.passerelle;", "    let gw = crate::net::gateway(); let _o = crate::net::our_ip();"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"configuration reseau : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"CONFIG_RESEAU_GENERATION_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
