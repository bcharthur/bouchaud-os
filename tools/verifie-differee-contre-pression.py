#!/usr/bin/env python3
"""Une ligne differee ne se perd pas tant que son producteur peut faire de la
place (P13_CONSOLE_DIFFEREE_CONTRE_PRESSION_V1).

Regles :

  1. `differe` retente la pose apres `fait_place_differee`, et ne compte une
     ligne perdue qu'apres un refus de celle-ci ou pour une ligne trop longue
     pour l'anneau du coeur ; repere et numero sont pris a chaque essai ;
  2. `fait_place_differee` renonce en panique, si CE coeur tient le jeton ou
     vide deja, et au-dela de `DIFFEREE_PLACE_BORNE_NS` (20 ms, la borne des
     contextes masques d'`aide_a_vider`) ;
  3. `fait_place_differee` ne dort ni ne cede jamais (contexte masque) : ni
     `sleep`, ni `wait`, ni `yield_now`, ni `schedule`, ni `point_cooperatif` ;
     elle n'emet que par `vide_un_morceau` (sous le jeton, rendu aussitot) ;
  4. la taille de l'anneau differe d'un coeur est inchangee (4096).

Fail-closed ; tests negatifs par mutation.
"""
import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
F = RACINE / "src/drivers/serial/uart16550.rs"
CARGO = RACINE / "Cargo.toml"
BANC = "banc-console-sans-contre-pression"


def corps(source: str, signature: str) -> str:
    debut = source.find(signature)
    if debut < 0:
        return ""
    ouverture = source.find("{", debut)
    profondeur = 0
    for i in range(ouverture, len(source)):
        if source[i] == "{":
            profondeur += 1
        elif source[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[ouverture:i + 1]
    return ""


def verifie(s: str, cargo: str) -> list:
    fautes = []
    d = corps(s, "fn differe(octets: &[u8]) {")
    boucle = d.find("loop {")
    numero = d.find("DIFFEREE_NUMERO.fetch_add(1")
    pose = d.find("DIFFEREES[cpu].range_morceaux(DIFFEREE_REPERE + octets.len()")
    place = d.find("if total + 2 > DIFFEREES_OCTETS || !fait_place_differee(debut) {")
    perdue = d.find("LIGNES_PERDUES.fetch_add(1")
    if not 0 <= boucle < numero < pose < place < perdue:
        fautes.append("differe : ligne perdue sans tentative de faire de la place")
    if d.count("LIGNES_PERDUES.fetch_add(1") != 1:
        fautes.append("differe : autre chemin de perte")
    f = corps(s, "fn fait_place_differee(debut: u64) -> bool {")
    for garde in ("EN_PANIQUE.load(Ordering::Acquire)", "EMISSION.load(Ordering::Acquire) == moi",
                  "VIDANGE_EN_COURS.load(Ordering::Acquire) == moi", "> DIFFEREE_PLACE_BORNE_NS"):
        if garde not in f:
            fautes.append(f"fait_place_differee : garde absente ({garde})")
    if re.search(r"sleep|wait|yield_now|schedule\(|point_cooperatif|ecris_octets|sous_jeton", f):
        fautes.append("fait_place_differee : dort, cede ou emet hors de vide_un_morceau")
    if "vide_un_morceau()" not in f:
        fautes.append("fait_place_differee : ne fait pas de place")
    if "const DIFFEREE_PLACE_BORNE_NS: u64 = 20_000_000;" not in s:
        fautes.append("borne de 20 ms modifiee")
    if "const DIFFEREES_OCTETS: usize = 4096;" not in s:
        fautes.append("taille de l'anneau differe modifiee : le correctif ne doit pas etre un tampon plus grand")
    # Banc temoin : le retrait de la contre-pression n'existe que sous l'option
    # de banc, jamais dans `default`, et seule la fonction de banc la retire.
    if f'#[cfg(not(feature = "{BANC}"))]\nfn fait_place_differee(debut: u64) -> bool {{' not in s:
        fautes.append("fait_place_differee : la contre-pression n'est pas la version de l'image livree")
    if s.count("fn fait_place_differee(") != 2 or \
            f'#[cfg(feature = "{BANC}")]\nfn fait_place_differee(_debut: u64) -> bool {{\n    false\n}}' not in s:
        fautes.append("banc temoin console : autre definition que le retrait sous option")
    defaut = re.search(r"^default = \[(.*)\]$", cargo, re.M)
    if not defaut or BANC in defaut.group(1) or f"\n{BANC} = []\n" not in cargo:
        fautes.append("banc temoin console dans `default`, ou option absente")
    return fautes


def negatifs(s: str, cargo: str) -> list:
    mutations = [
        ("perte immediate", "if total + 2 > DIFFEREES_OCTETS || !fait_place_differee(debut) {",
         "if true {"),
        ("sans garde panique", "    if EN_PANIQUE.load(Ordering::Acquire) {\n        return false;\n    }\n    let moi", "    let moi"),
        ("jeton de ce coeur ignore", "EMISSION.load(Ordering::Acquire) == moi ||", ""),
        ("sans borne", "> DIFFEREE_PLACE_BORNE_NS", "> u64::MAX"),
        ("cede en contexte masque", "    if let Vidange::JetonOccupe = vide_un_morceau() {\n        core::hint::spin_loop();",
         "    if let Vidange::JetonOccupe = vide_un_morceau() {\n        crate::kernel::task::yield_now();"),
        ("tampon agrandi", "const DIFFEREES_OCTETS: usize = 4096;", "const DIFFEREES_OCTETS: usize = 65536;"),
        ("temoin dans l'image livree", f'#[cfg(not(feature = "{BANC}"))]\nfn fait_place_differee(debut', "fn fait_place_differee(debut"),
    ]
    echecs = []
    for nom, avant, apres in mutations:
        if avant not in s:
            echecs.append(f"mutation inapplicable : {nom}")
            continue
        if not verifie(s.replace(avant, apres, 1), cargo):
            echecs.append(f"mutation non detectee : {nom}")
    if not verifie(s, cargo.replace('default = ["legacy-boot"]', f'default = ["legacy-boot", "{BANC}"]', 1)):
        echecs.append("mutation non detectee : temoin dans default")
    return echecs


def main() -> int:
    s = F.read_text(encoding="utf-8")
    cargo = CARGO.read_text(encoding="utf-8")
    fautes = verifie(s, cargo) + negatifs(s, cargo)
    for faute in fautes:
        print(f"ECHEC {faute}")
    if fautes:
        return 1
    print("lignes differees : contre-pression sans sommeil, bornee : ok (8 mutations detectees)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
