#!/usr/bin/env python3
"""La sortante d'une commutation, remise en file sur le coeur qui vient
d'elire, ne pose aucune demande de replanification (P13_REMISE_SANS_DEMANDE_V1).

Regles :

  1. `reveil::demande_differee_posee` : seule une `Remise` sur le coeur local
     ne pose pas la demande differee (test hote `test_reveil_cible.rs`) ;
  2. `publie_pret`, branche `DemandeDifferee` : `request_cpu` n'est appele que
     si `demande_differee_posee(publication, target == local_cpu())` ; l'IPI
     au coeur endormi reste inconditionnel a la decision ;
  3. `publish_ready` publie un `Reveil` -- toutes les publications sauf une ;
  4. `prepare_switch_handoff` retient si la sortante etait PRETE (preemptee ou
     cedee) ; `complete_switch_handoff` lit ce drapeau AVANT de rendre
     `SWITCH_PENDING`, et ne publie une `Remise` que dans ce cas -- une
     sortante rendue prete pendant la commutation reste un `Reveil` ;
  5. `Remise` n'apparait nulle part ailleurs dans le noyau.

Fail-closed ; tests negatifs par mutation.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
F = {
    "pur": RACINE / "src/kernel/scheduler/reveil.rs",
    "creation": RACINE / "src/kernel/process/thread/creation.rs",
    "courant": RACINE / "src/kernel/process/thread/courant.rs",
    "test": RACINE / "tools/smp/test_reveil_cible.rs",
}
REGLE = "publication == Publication::Reveil || !cible_locale"
GARDE = ("if crate::kernel::scheduler::reveil::demande_differee_posee(publication, target == local_cpu()) {\n"
         "                crate::kernel::scheduler::preempt::request_cpu(target);")
RETENUE = "SWITCH_REMISE[cpu].store(task.state == TaskState::Ready, Ordering::Relaxed);"
LECTURE = "let remise = SWITCH_REMISE[cpu].load(Ordering::Relaxed);"
RENDU = "SWITCH_PENDING[cpu].store(NO_TASK, Ordering::Release);"
PUBLIE = "publie_pret(outgoing, if remise { ReveilPublication::Remise } else { ReveilPublication::Reveil });"


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


def verifie(t: dict, autres: str) -> list:
    fautes = []
    if corps(t["pur"], "pub fn demande_differee_posee(").strip("{} \n") != REGLE:
        fautes.append("reveil.rs : la regle n'est plus celle testee")
    publie = corps(t["creation"], "fn publie_pret(")
    differee = publie.find("ReveilDecision::DemandeDifferee =>")
    suivante = publie.find("ReveilDecision::MiseEnFile =>")
    branche = publie[differee:suivante] if 0 <= differee < suivante else ""
    if GARDE not in branche or branche.count("request_cpu(") != 1:
        fautes.append("publie_pret : la demande differee est posee sans la regle de remise")
    if "if etat.inactif && !reveil_servi_localement(target) {" not in branche:
        fautes.append("publie_pret : l'IPI au coeur endormi depend de la remise")
    if corps(t["creation"], "fn publish_ready(").strip("{} \n") != "publie_pret(index, ReveilPublication::Reveil);":
        fautes.append("publish_ready : une publication ordinaire n'est plus un reveil")
    prepare = corps(t["courant"], "fn prepare_switch_handoff(")
    if RETENUE not in prepare:
        fautes.append("prepare_switch_handoff : la sortante prete n'est pas retenue")
    complete = corps(t["courant"], "fn complete_switch_handoff(")
    if not 0 <= complete.find(LECTURE) < complete.find(RENDU) < complete.find(PUBLIE):
        fautes.append("complete_switch_handoff : remise lue apres le rendu de la case, ou publiee sans elle")
    if (t["creation"] + t["courant"] + autres).count("ReveilPublication::Remise") != 1:
        fautes.append("une Remise est publiee hors de complete_switch_handoff")
    if "fn la_remise_ne_transforme_plus_chaque_appel_systeme_en_commutation()" not in t["test"]:
        fautes.append("test hote absent")
    return fautes


def negatifs(t: dict, autres: str) -> list:
    mutations = [
        ("remise demande toujours", "pur", REGLE, "true"),
        ("remise ailleurs sans demande", "pur", REGLE, "publication == Publication::Reveil"),
        ("demande sans regle", "creation", GARDE,
         "if true {\n                crate::kernel::scheduler::preempt::request_cpu(target);"),
        ("coeur cible pris pour local", "creation", "publication, target == local_cpu())", "publication, true)"),
        ("publication ordinaire en remise", "creation", "publie_pret(index, ReveilPublication::Reveil);",
         "publie_pret(index, ReveilPublication::Remise);"),
        ("sortante bloquee prise pour remise", "courant", RETENUE,
         "SWITCH_REMISE[cpu].store(true, Ordering::Relaxed);"),
        ("remise lue apres rendu", "courant", LECTURE + "\n    " + RENDU, RENDU + "\n    " + LECTURE),
        ("remise inconditionnelle", "courant", PUBLIE,
         "publie_pret(outgoing, ReveilPublication::Remise);"),
    ]
    echecs = []
    for nom, cle, avant, apres in mutations:
        if avant not in t[cle]:
            echecs.append(f"mutation inapplicable : {nom}")
            continue
        m = dict(t)
        m[cle] = m[cle].replace(avant, apres, 1)
        if not verifie(m, autres):
            echecs.append(f"mutation non detectee : {nom}")
    if not verifie(t, autres + "\nfn x() { publie_pret(0, ReveilPublication::Remise); }\n"):
        echecs.append("mutation non detectee : remise publiee ailleurs")
    return echecs


def main() -> int:
    t = {cle: p.read_text(encoding="utf-8") for cle, p in F.items()}
    exclus = {F["creation"], F["courant"]}
    autres = "\n".join(p.read_text(encoding="utf-8") for p in sorted((RACINE / "src").rglob("*.rs")) if p not in exclus)
    fautes = verifie(t, autres) + negatifs(t, autres)
    for faute in fautes:
        print(f"ECHEC {faute}")
    if fautes:
        return 1
    print("remise de la sortante sans demande : ok (9 mutations detectees)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
