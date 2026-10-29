#!/usr/bin/env python3
"""Un tir perdu du minuteur local ne le tue pas (P13_MINUTEUR_PERIME_V1, C23).

Confirmation stricte 40812095472 (2d850401), A/B reveil precis KVM SMP1,
tour 4 `on` : `deadline_tsc=0 arme_tsc=286824508404` fige d'un releve a
l'autre, `tirs=+0 refus_quantum_avant=+4656`. Le quantum ne se rearme qu'au
vecteur local ; tout armement precis se jugeait contre l'echeance morte.

Regles :

  1. `porte_vivante` : une echeance passee ne porte plus rien (`u64::MAX`) ;
     `minuteur_perime` : armee, depassee de PLUS d'un quantum ;
  2. smp.rs : la decision du reveil precis et l'avance d'un sommeil
     interactif se jugent contre `porte(...)` ; a l'entree d'idle, AVANT
     l'armement precis (donc politique active ou non), un minuteur perime
     rearme le quantum et est compte ;
  3. le temoin `banc-minuteur-historique` (comportement d'avant : rien n'est
     releve, l'echeance morte reste la borne) n'existe que sous son option,
     absente de `default`, avec sa banniere ;
  4. le banc : injection verifiee (sinon NON_QUALIFIANT, jamais vert),
     exigences (quantum, tirs, vivant) jugeant le defaut, `releves` publie
     (un des deux chemins du correctif) ; p13-cible-kvm exige ECHEC
     du temoin et joue la livree en SMP1 et SMP4 ;
  5. tests hote presents.

Fail-closed ; tests negatifs par mutation.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
F = {
    "pur": RACINE / "src/kernel/time/reveil_precis.rs",
    "smp": RACINE / "src/arch/x86_64/smp.rs",
    "cargo": RACINE / "Cargo.toml",
    "main": RACINE / "src/main.rs",
    "test": RACINE / "tools/smp/test_reveil_precis.rs",
    "banc": RACINE / "tools/ci/run_minuteur_perdu.sh",
    "ci": RACINE / ".github/workflows/p13-cible-kvm.yml",
}
OPTION = 'feature = "banc-minuteur-historique"'
PORTE = ("pub fn porte_vivante(arme_tsc: u64, tsc_maintenant: u64) -> u64 {\n"
         "    if arme_tsc > tsc_maintenant { arme_tsc } else { u64::MAX }\n}")
PERIME = ("pub fn minuteur_perime(etat: EtatDeadline, arme_tsc: u64, tsc_maintenant: u64, quantum_cycles: u64) -> bool {\n"
          "    etat.arme() && quantum_cycles != 0 && arme_tsc.saturating_add(quantum_cycles) < tsc_maintenant\n}")
DECISION = "        porte(ARME_TSC[cpu].load(Ordering::Relaxed), crate::arch::x86_64::cpu::rdtsc()),"
AVANCE = ("        if cible < porte(ARME_TSC[cpu].load(Ordering::Relaxed), maintenant)\n"
          "            && cible < porte(QUANTUM_TSC[cpu].load(Ordering::Relaxed), maintenant)")
ORDRE_IDLE = "    releve_minuteur_perime(cpu);\n    arme_reveil_precis_sous_drapeau(cpu);"
RELEVE_SI = ("    if crate::kernel::reveil_precis::minuteur_perime(etat, ARME_TSC[cpu].load(Ordering::Relaxed),\n"
             "        crate::arch::x86_64::cpu::rdtsc(), quantum)\n    {\n"
             "        MINUTEURS_PERIMES.fetch_add(1, Ordering::Relaxed);\n        arm_local_scheduler_timer();\n    }")
QUANTUM = "    let quantum = crate::kernel::timer::cycles_par_ms().saturating_mul(SCHED_QUANTUM_TICKS);"
TEMOIN_RELEVE = f'#[cfg({OPTION})]\nfn releve_minuteur_perime(_cpu: usize) {{}}'
TEMOIN_PORTE = f'#[cfg({OPTION})]\nfn porte(arme_tsc: u64, _tsc_maintenant: u64) -> u64 {{\n    arme_tsc\n}}'
VRAI_RELEVE = f'#[cfg(not({OPTION}))]\nfn releve_minuteur_perime(cpu: usize) {{'
VRAIE_PORTE = (f'#[cfg(not({OPTION}))]\nfn porte(arme_tsc: u64, tsc_maintenant: u64) -> u64 {{\n'
               '    crate::kernel::reveil_precis::porte_vivante(arme_tsc, tsc_maintenant)\n}')
BANNIERE_BANC = ('    #[cfg(feature = "banc-minuteur-perdu")]\n'
                 '    crate::serial_println!("BOUCHAUD_IMAGE_TEMOIN feature=banc-minuteur-perdu livrable=non qualifiable=non");')
BANNIERE = (f'    #[cfg({OPTION})]\n'
            '    crate::serial_println!("BOUCHAUD_IMAGE_TEMOIN feature=banc-minuteur-historique livrable=non qualifiable=non");')
TESTS = ("une_echeance_passee_ne_porte_plus_rien", "le_cas_observe_est_arme_avec_la_porte_vivante_et_refuse_sans",
         "un_quantum_vivant_plus_proche_reste_arme", "minuteur_perime_seulement_au_dela_d_un_quantum_et_arme")
NQ = ('[ "$(champ injection_tenue)" = 1 ] || nq="$nq injection-non-tenue"',
      '[ "$(champ quantums_pendant_injection)" = 0 ] || nq="$nq quantum-pendant-injection"',
      '  echo "MINUTEUR_PERDU_NON_QUALIFIANT raisons=${nq# }"\n  exit 2')
EXIGENCES = ('echo "  info    minuteurs releves a l\'inactivite : $(champ releves)"',
             'exige "le quantum bat de nouveau (quantums > 0)" test "$(champ quantums)" -gt 0',
             'exige "les tirs precis reprennent (tirs > 0)" test "$(champ tirs)" -gt 0',
             'exige "le minuteur est vivant a la fin" test "$(champ vivant_a_la_fin)" = 1')
REMESURE = '    if [ "$code" != 2 ]; then exit "$code"; fi'
CI_TEMOIN = 'if [ "$code" != 1 ]; then echo "TEMOIN_MINUTEUR code=$code (ECHEC attendu : 1)" >&2; exit 1; fi'
CI_TEMOIN_ARME = "BANC_TENTATIVES=3 BO_SMP=1 BO_QEMU_KVM=1 tools/ci/run_minuteur_perdu.sh boot-temoin.bin 3000"
CI_LIVREE = ("          for smp in 1 4; do for i in 1 2 3; do\n"
             "            BANC_TENTATIVES=3 BO_SMP=$smp BO_QEMU_KVM=1 tools/ci/run_minuteur_perdu.sh boot-banc.bin 3000 || r=1")
CI_IMAGE = "CARGO_TARGET_DIR=target-temoin cargo bootimage --features banc-minuteur-historique"
CI_BANC = "CARGO_TARGET_DIR=target-banc cargo bootimage --features banc-minuteur-perdu"
CI_LIVREE_PROPRE = "if grep -aq BOUCHAUD_IMAGE_TEMOIN boot-livree.bin; then echo BANNIERE_TEMOIN_DANS_LA_LIVREE >&2; exit 1; fi"


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
                return source[debut:i + 1]
    return ""


def defaut(cargo: str) -> str:
    for ligne in cargo.splitlines():
        if ligne.strip().startswith("default"):
            return ligne
    return ""


def verifie(t: dict) -> list:
    fautes = []
    if PORTE not in t["pur"] or PERIME not in t["pur"]:
        fautes.append("reveil_precis.rs : `porte_vivante` ou `minuteur_perime` a change.")
    s = t["smp"]
    if DECISION not in corps(s, "fn arme_reveil_precis_sous_drapeau("):
        fautes.append("smp.rs : la decision du reveil precis ne se juge plus contre la porte vivante.")
    if AVANCE not in corps(s, "pub fn avance_pour_sommeil_interactif("):
        fautes.append("smp.rs : l'avance d'un sommeil interactif ne se juge plus contre la porte vivante.")
    if ORDRE_IDLE not in corps(s, "pub fn arme_reveil_precis_avant_hlt("):
        fautes.append("smp.rs : le minuteur perime n'est plus releve a l'entree d'idle, avant l'armement precis.")
    vrai = corps(s, VRAI_RELEVE)
    if not vrai or RELEVE_SI not in vrai or QUANTUM not in vrai:
        fautes.append("smp.rs : un minuteur perime ne rearme plus le quantum, ou n'est plus compte.")
    if VRAIE_PORTE not in s:
        fautes.append("smp.rs : la porte de la livree n'est plus `porte_vivante`.")
    if TEMOIN_RELEVE not in s or TEMOIN_PORTE not in s:
        fautes.append("smp.rs : le temoin historique n'est plus sous son option.")
    if s.count("fn releve_minuteur_perime(") != 2 or s.count("fn porte(") != 2:
        fautes.append("smp.rs : un releve ou une porte hors des deux variantes.")
    if 'banc-minuteur-perdu = []' not in t["cargo"] \
            or 'banc-minuteur-historique = ["banc-minuteur-perdu"]' not in t["cargo"] \
            or "banc-minuteur" in defaut(t["cargo"]):
        fautes.append("Cargo.toml : les options de banc manquent ou sont dans `default`.")
    if '#[cfg(feature = "banc-minuteur-perdu")]\npub fn banc_perd_tir() -> Option<u64> {' not in s:
        fautes.append("smp.rs : l'injection du tir perdu est compilee hors de son option de banc.")
    if BANNIERE_BANC not in t["main"]:
        fautes.append("main.rs : la banniere du banc n'est plus compilee sous son option seule.")
    if BANNIERE not in t["main"]:
        fautes.append("main.rs : la banniere temoin n'est plus compilee sous l'option seule.")
    for nom in TESTS:
        if f"fn {nom}()" not in t["test"]:
            fautes.append(f"tests hote : `{nom}` absent.")
    for extrait in NQ + EXIGENCES + (REMESURE,):
        if extrait not in t["banc"]:
            fautes.append(f"run_minuteur_perdu.sh : extrait absent ou change : {extrait.splitlines()[0][:70]}")
    for extrait in (CI_TEMOIN, CI_TEMOIN_ARME, CI_LIVREE, CI_IMAGE, CI_BANC, CI_LIVREE_PROPRE):
        if extrait not in t["ci"]:
            fautes.append(f"p13-cible-kvm.yml : extrait absent ou change : {extrait.splitlines()[0][:70]}")
    return fautes


def negatifs(t: dict) -> tuple:
    mutations = [
        ("echeance passee portee", "pur", "if arme_tsc > tsc_maintenant {", "if true {"),
        ("perime des la premiere cycle", "pur", "arme_tsc.saturating_add(quantum_cycles) < tsc_maintenant",
         "arme_tsc < tsc_maintenant"),
        ("perime sans etat arme", "pur", "    etat.arme() && quantum_cycles != 0 &&", "    quantum_cycles != 0 &&"),
        ("decision sur l'echeance brute", "smp", DECISION, "        ARME_TSC[cpu].load(Ordering::Relaxed),"),
        ("avance sur l'echeance brute", "smp", AVANCE,
         "        if cible < ARME_TSC[cpu].load(Ordering::Relaxed) && cible < QUANTUM_TSC[cpu].load(Ordering::Relaxed)"),
        ("releve apres l'armement", "smp", ORDRE_IDLE, "    arme_reveil_precis_sous_drapeau(cpu);\n    releve_minuteur_perime(cpu);"),
        ("releve oublie", "smp", ORDRE_IDLE, "    arme_reveil_precis_sous_drapeau(cpu);"),
        ("releve sans rearmement", "smp", "        MINUTEURS_PERIMES.fetch_add(1, Ordering::Relaxed);\n        arm_local_scheduler_timer();",
         "        MINUTEURS_PERIMES.fetch_add(1, Ordering::Relaxed);"),
        ("releve non compte", "smp", "        MINUTEURS_PERIMES.fetch_add(1, Ordering::Relaxed);\n        arm_local_scheduler_timer();",
         "        arm_local_scheduler_timer();"),
        ("porte livree historique", "smp", "    crate::kernel::reveil_precis::porte_vivante(arme_tsc, tsc_maintenant)\n}",
         "    let _ = tsc_maintenant;\n    arme_tsc\n}"),
        ("temoin hors option", "smp", TEMOIN_RELEVE, "fn releve_minuteur_perime(_cpu: usize) {}"),
        ("option par defaut", "cargo", 'default = ["legacy-boot"]', 'default = ["legacy-boot", "banc-minuteur-historique"]'),
        ("injection dans la livree", "smp", '#[cfg(feature = "banc-minuteur-perdu")]\npub fn banc_perd_tir()', 'pub fn banc_perd_tir()'),
        ("banc sur la livree", "ci", "run_minuteur_perdu.sh boot-banc.bin 3000 || r=1", "run_minuteur_perdu.sh boot-livree.bin 3000 || r=1"),
        ("banniere du banc dans la livree", "main", BANNIERE_BANC, BANNIERE_BANC.replace('    #[cfg(feature = "banc-minuteur-perdu")]\n', "")),
        ("banniere sans option", "main", BANNIERE, BANNIERE.replace(f"    #[cfg({OPTION})]\n", "")),
        ("test retire", "test", "fn le_cas_observe_est_arme_avec_la_porte_vivante_et_refuse_sans()", "fn x()"),
        ("injection non verifiee", "banc", NQ[0], "true"),
        ("non qualifiant vert", "banc", NQ[2], '  echo "MINUTEUR_PERDU_NON_QUALIFIANT raisons=${nq# }"\n  exit 0'),
        ("releve non publie", "banc", EXIGENCES[0], "true"),
        ("quantum non exige", "banc", EXIGENCES[1], "true"),
        ("vivant non exige", "banc", EXIGENCES[3], "true"),
        ("tirs non exiges", "banc", EXIGENCES[2], "true"),
        ("echec remesure", "banc", REMESURE, '    if [ "$code" = 0 ]; then exit 0; fi'),
        ("temoin accepte vert", "ci", CI_TEMOIN, 'if [ "$code" = 2 ]; then exit 1; fi'),
        ("livree sans smp4", "ci", CI_LIVREE, CI_LIVREE.replace("for smp in 1 4;", "for smp in 1;")),
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
            print(f"verifie-minuteur-perime: {f}")
        return 1
    print(f"verifie-minuteur-perime: OK (echeance passee sans portee, minuteur perime releve a l'idle, "
          f"temoin sous option, banc et CI ; {n} negatifs)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
