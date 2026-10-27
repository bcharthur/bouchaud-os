#!/usr/bin/env python3
"""Une tache reveillee ne reste pas en file sur un coeur qui n'elira pas
(P13_REVEIL_COEUR_MUET_V1).

Le reveil etait « perdu » entre la demande et la reprise : publiee sur son
coeur historique avec une demande differee, la tache attendait un occupant
qui ne cedait pas -- shell hors ordonnanceur, fil noyau masque, vCPU retire --
et, fil noyau, n'etait volee par personne.

Regles :

  1. `portee_replacement` : privilege OU historique hors ordonnanceur -> tout
     coeur ; historique tenu par un fil noyau (et pas au repos) -> un coeur
     au repos seulement ; sinon l'historique ;
  2. `Selection` en portee `Inactifs` ne retient qu'un coeur au repos qui
     elit (pas un BSP hors ordonnanceur) ;
  3. `publie_pret` passe par `portee_replacement` et `avec_portee`, et
     `etat_coeur_reveil` renseigne `occupant_noyau` depuis CURRENT_IS_KERNEL ;
  4. `runnable_steal` accepte un fil noyau MIGRABLE, jamais hors affinite ;
  5. le bras temoin `banc-reveil-historique` n'existe que sous son option,
     absente de `default` ;
  6. tests hote presents ; l'assertion qui consacrait le defaut (une normale
     garde un historique hors ordonnanceur) n'existe plus ;
  7. IMAGE TEMOIN (option 2) : la banniere BOUCHAUD_IMAGE_TEMOIN n'est
     compilee que sous l'option ; tout workflow qui construit
     `banc-reveil-historique` le fait dans `target-temoin` ; os-primitives
     verifie l'image qualifiee inchangee et sans banniere, joue les quatre
     bancs « coeur du fil » sur l'image temoin, exige OK du banc de reveil sur
     l'image qualifiee et ECHEC (code 1, jamais 2) sur le temoin, et publie
     les preuves temoin dans un artefact a part.

Fail-closed ; tests negatifs par mutation.
"""
import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
F = {
    "reveil": RACINE / "src/kernel/scheduler/reveil.rs",
    "creation": RACINE / "src/kernel/process/thread/creation.rs",
    "ordo": RACINE / "src/kernel/process/thread/ordonnancement.rs",
    "test": RACINE / "tools/smp/test_reveil_cible.rs",
    "cargo": RACINE / "Cargo.toml",
    "main": RACINE / "src/main.rs",
    "osp": RACINE / ".github/workflows/os-primitives.yml",
    "banc": RACINE / "tools/ci/run_reveil_coeur_muet.sh",
}
WORKFLOWS = RACINE / ".github/workflows"
OFF = "target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin"
BANNIERE = ('#[cfg(feature = "banc-reveil-historique")]\n'
            '    crate::serial_println!("BOUCHAUD_IMAGE_TEMOIN feature=banc-reveil-historique livrable=non qualifiable=non");')
BANC = "banc-reveil-historique"
PORTEE = (f'#[cfg(not(feature = "{BANC}"))]\n'
          "pub fn portee_replacement(reveille: &Reveille, precedent: &Coeur) -> Option<Portee> {\n"
          "    if reveille.privilegiee() || precedent.hors_ordonnanceur {\n"
          "        Some(Portee::Tous)\n"
          "    } else if precedent.occupant_noyau && !precedent.inactif {\n"
          "        Some(Portee::Inactifs)\n"
          "    } else {\n"
          "        None\n"
          "    }\n"
          "}")
FILTRE = ("        if self.inactifs_seulement && (!coeur.inactif || coeur.hors_ordonnanceur) {\n"
          "            return;\n"
          "        }")
OCCUPANT = ("        occupant_noyau: en_ligne && occupant_coeur(cpu) != NO_TASK\n"
            "            && CURRENT_IS_KERNEL[cpu].load(Ordering::Acquire),")
VOL = (f'#[cfg(not(feature = "{BANC}"))]\n'
       "fn runnable_steal(task: &Task, cpu: usize) -> bool {\n"
       "    task.state == TaskState::Ready\n"
       "        && task.on_cpu < 0\n"
       "        && !task.switching_out.charge()\n"
       "        && (!task.noyau || task.migrable)\n"
       "        && task.runq_cpu.charge() as usize != cpu\n"
       "        && allowed_on(task, cpu)\n"
       "}")
DEFAUT = "assert_eq!(choisit_coeur(0, &normal, &[bsp, coeur_libre()]), None);"


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


def verifie(t: dict) -> list:
    f = []
    if PORTEE not in t["reveil"]:
        f.append("portee_replacement : la regle livree n'est plus celle testee")
    if t["reveil"].count("pub fn portee_replacement(") != 2 or t["reveil"].count(f'#[cfg(feature = "{BANC}")]') != 1:
        f.append("portee_replacement : autre definition que le temoin sous option")
    if FILTRE not in corps(t["reveil"], "pub fn propose(&mut self, indice: usize, coeur: &Coeur)"):
        f.append("Selection : la portee Inactifs retient un coeur occupe ou hors ordonnanceur")
    publie = corps(t["creation"], "fn publie_pret(")
    if "reveil::portee_replacement(" not in publie or "ReveilSelection::avec_portee(precedent, portee)" not in publie \
            or "ReveilSelection::neuve(" in publie:
        f.append("publie_pret : ne suit pas la portee de replacement")
    if OCCUPANT not in corps(t["creation"], "fn etat_coeur_reveil("):
        f.append("etat_coeur_reveil : occupant_noyau non renseigne")
    if VOL not in t["ordo"] or t["ordo"].count("fn runnable_steal(") != 2:
        f.append("runnable_steal : fil noyau migrable non volable, ou vol hors affinite")
    defaut = re.search(r"^default = \[(.*)\]$", t["cargo"], re.M)
    if not defaut or BANC in defaut.group(1) or f"\n{BANC} = []\n" not in t["cargo"]:
        f.append("banc temoin reveil dans `default`, ou option absente")
    for test in ("fn une_normale_quitte_un_historique_hors_ordonnanceur()",
                 "fn un_historique_tenu_par_un_fil_noyau_cede_la_place_a_un_coeur_au_repos()",
                 "fn portee_du_replacement()"):
        if test not in t["test"]:
            f.append(f"test hote absent : {test}")
    code_test = "\n".join(l for l in t["test"].split("\n") if not l.lstrip().startswith("//"))
    if DEFAUT in code_test:
        f.append("test hote : l'assertion qui consacrait le defaut est revenue")
    # 7. image temoin
    if BANNIERE not in t["main"] or t["main"].count("BOUCHAUD_IMAGE_TEMOIN feature=banc-reveil-historique") != 1:
        f.append("main.rs : banniere temoin absente ou hors de son option")
    flux = dict(t["workflows"])
    flux["os-primitives.yml"] = t["osp"]
    for nom, texte in flux.items():
        for ligne in texte.split("\n"):
            if "--features banc-reveil-historique" in ligne and "CARGO_TARGET_DIR=target-temoin" not in ligne:
                f.append(f"{nom} : image temoin construite dans le repertoire de l'image qualifiee")
    osp = t["osp"]
    for attendu in ('[ "$avant" = "$apres" ] || { echo "IMAGE_QUALIFIEE_MODIFIEE_PAR_LE_TEMOIN"',
                    f"if grep -aq BOUCHAUD_IMAGE_TEMOIN {OFF}; then",
                    "if [ \"$code\" != 1 ]; then echo \"TEMOIN_REVEIL_$nom",
                    f"BO_QEMU_KVM=$accel BO_SMP=$smp tools/ci/run_reveil_coeur_muet.sh {OFF} 3000 || r=1",
                    "name: os-primitives-temoin-reveil-historique"):
        if attendu not in osp:
            f.append(f"os-primitives : protocole temoin incomplet ({attendu[:50]})")
    fil = [l for l in osp.split("\n") if "BANC_VERROU_CIBLE=fil" in l and "run_serie_entrelacement.sh" in l]
    if len(fil) != 4 or any('run_serie_entrelacement.sh "$T"' not in l for l in fil):
        f.append("os-primitives : un banc « coeur du fil » ne tourne pas sur l'image temoin")
    banc = t["banc"]
    for attendu in ('echo "REVEIL_COEUR_MUET_NON_QUALIFIANT raisons=${nq# }"\n  exit 2',
                    'exige "aucune double election du dormeur"', 'exige "le fil epingle n\'a jamais tourne hors de son coeur"'):
        if attendu not in banc:
            f.append(f"banc de reveil : exigence ou non-qualifiant absent ({attendu[:40]})")
    return f


def negatifs(t: dict) -> tuple:
    mutations = [
        ("hors ordonnanceur reserve aux interactives", "reveil",
         "    if reveille.privilegiee() || precedent.hors_ordonnanceur {\n        Some(Portee::Tous)",
         "    if reveille.privilegiee() || (reveille.classe == Classe::Interactive && precedent.hors_ordonnanceur) {\n        Some(Portee::Tous)"),
        ("fil noyau occupant ignore", "reveil",
         "    } else if precedent.occupant_noyau && !precedent.inactif {", "    } else if false {"),
        ("vers un coeur occupe", "reveil", "        Some(Portee::Inactifs)\n    } else {", "        Some(Portee::Tous)\n    } else {"),
        ("portee Inactifs sans filtre", "reveil", FILTRE, ""),
        ("publication sur l'historique", "creation", "ReveilSelection::avec_portee(precedent, portee)",
         "ReveilSelection::neuve(precedent, true)"),
        ("occupant noyau jamais vu", "creation", OCCUPANT, "        occupant_noyau: false,"),
        ("fil noyau jamais vole", "ordo", "        && (!task.noyau || task.migrable)\n        && task.runq_cpu.charge() as usize != cpu\n        && allowed_on(task, cpu)\n}",
         "        && !task.noyau\n        && task.runq_cpu.charge() as usize != cpu\n        && allowed_on(task, cpu)\n}"),
        ("vol hors affinite", "ordo", "        && (!task.noyau || task.migrable)\n        && task.runq_cpu.charge() as usize != cpu\n        && allowed_on(task, cpu)\n}",
         "        && (!task.noyau || task.migrable)\n        && task.runq_cpu.charge() as usize != cpu\n}"),
        ("temoin dans default", "cargo", 'default = ["legacy-boot"]', f'default = ["legacy-boot", "{BANC}"]'),
        ("banniere temoin dans l'image livree", "main", BANNIERE,
         '    crate::serial_println!("BOUCHAUD_IMAGE_TEMOIN feature=banc-reveil-historique livrable=non qualifiable=non");'),
        ("temoin construit sur l'image qualifiee", "osp", "CARGO_TARGET_DIR=target-temoin cargo bootimage --features banc-reveil-historique",
         "cargo bootimage --features banc-reveil-historique"),
        ("qualifiee modifiee toleree", "osp", '[ "$avant" = "$apres" ] || { echo "IMAGE_QUALIFIEE_MODIFIEE_PAR_LE_TEMOIN" >&2; exit 1; }', "true"),
        ("coeur du fil sur l'image qualifiee", "osp",
         'BANC_VERROU_SUSPENDU_MS=3000 BANC_VERROU_CIBLE=fil BANC_SECOURS=off tools/ci/run_serie_entrelacement.sh "$T" || r=1\n          cp serie-entrelacement.log temoin-reveil-historique/serie-entrelacement-tcg',
         f'BANC_VERROU_SUSPENDU_MS=3000 BANC_VERROU_CIBLE=fil BANC_SECOURS=off tools/ci/run_serie_entrelacement.sh {OFF} || r=1\n          cp serie-entrelacement.log temoin-reveil-historique/serie-entrelacement-tcg'),
        ("temoin non qualifiant accepte", "osp", 'if [ "$code" != 1 ]; then', 'if [ "$code" = 0 ]; then'),
        ("non-qualifiant vert", "banc", 'echo "REVEIL_COEUR_MUET_NON_QUALIFIANT raisons=${nq# }"\n  exit 2',
         'echo "REVEIL_COEUR_MUET_NON_QUALIFIANT raisons=${nq# }"\n  exit 0'),
        ("assertion du defaut retablie", "test",
         "fn interactive_ordinaire_garde_un_coeur_historique_disponible() {\n",
         "fn interactive_ordinaire_garde_un_coeur_historique_disponible() {\n    let bsp = Coeur { hors_ordonnanceur: true, ..coeur_libre() };\n"
         "    let normal = Reveille { classe: Classe::Normale, ..ordinaire() };\n    " + DEFAUT + "\n"),
    ]
    echecs = []
    for nom, cle, avant, apres in mutations:
        if t[cle].count(avant) != 1:
            echecs.append(f"mutation inapplicable ({t[cle].count(avant)}) : {nom}")
            continue
        m = dict(t)
        m[cle] = m[cle].replace(avant, apres, 1)
        if not verifie(m):
            echecs.append(f"mutation non detectee : {nom}")
    return echecs, len(mutations)


def main() -> int:
    t = {cle: p.read_text(encoding="utf-8") for cle, p in F.items()}
    t["workflows"] = {p.name: p.read_text(encoding="utf-8") for p in sorted(WORKFLOWS.glob("*.yml"))}
    fautes = verifie(t)
    echecs, n = negatifs(t)
    for faute in fautes + echecs:
        print(f"ECHEC {faute}")
    if fautes or echecs:
        return 1
    print(f"reveil vers un coeur qui elit : ok ({n} mutations detectees)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
