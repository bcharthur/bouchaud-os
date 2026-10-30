#!/usr/bin/env python3
"""Garde C22 V2 : les quatre heaps mimalloc sont possedes PAR FIL.

La V1 verifiait un pointeur global + owner pthread et refusait donc le
correctif V2. Cette garde controle la nouvelle invariance et ses mutations
negatives, sans affaiblir le temoin historique ni l'appel Document::destroy.
"""
import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
PREP = "tools/ladybird/prepare-c22-partitions.py"
CHAINE = "tools/ladybird/browser-upstream.sh"


def verifie(prep: str, chaine: str) -> list[str]:
    fautes = []
    for nom, index in (
        ("array_buffer_heap", 0),
        ("js_object_storage_heap", 1),
        ("layout_heap", 2),
        ("painting_heap", 3),
    ):
        if f'("{nom}", {index})' not in prep:
            fautes.append(f"tas {nom} absent ou index {index} change")

    if "static thread_local mi_heap_t* s_bouchaud_partitions[4]" not in prep:
        fautes.append("heaps collectes non locaux au fil")
    if "static thread_local mi_heap_t* {nom}" not in prep:
        fautes.append("heaps alloues non locaux au fil")
    if "s_bouchaud_partitions[index] = heap;" not in prep:
        fautes.append("heap non enregistre dans le fil createur")
    if "for (auto* heap : s_bouchaud_partitions)" not in prep:
        fautes.append("collecte ne parcourt pas les heaps du fil")
    if "mi_heap_collect(heap, true);" not in prep:
        fautes.append("collecte forte de heap absente")
    if "std::atomic<mi_heap_t*>" in prep or "pthread_equal" in prep:
        fautes.append("ancien partage global du heap interdit")

    kmalloc, sep, doc = prep.partition('    # BOUCHAUD_C22_DOCUMENT_COLLECTE_V2')
    if not sep:
        fautes.append("section Document::destroy absente")
    if "BOUCHAUD_LB_C22_TEMOIN_HISTORIQUE" not in kmalloc or kmalloc.count("if (!temoin) {") != 1:
        fautes.append("temoin historique AK non protege")
    if "mi_collect(true);" not in kmalloc:
        fautes.append("collecte du heap par defaut absente")
    if "[LB:C22] collecte partitions=%d temoin=%d" not in kmalloc:
        fautes.append("preuve de collecte des heaps absente")
    if re.search(r"mi_collect\(false\)|mi_heap_destroy|mi_heap_delete", prep):
        fautes.append("operation de destruction ou collecte faible interdite")

    if "Libraries/LibWeb/DOM/Document.cpp" not in doc:
        fautes.append("Document.cpp non patche")
    if "BOUCHAUD_LB_C22_TEMOIN_HISTORIQUE" not in doc or doc.count("if (!temoin) {") != 1:
        fautes.append("temoin historique Document non protege")
    if "ak_kmalloc_collect();" not in doc:
        fautes.append("collecte apres Document::destroy absente")
    if "[LB:C22] document_destroy collecte duree_us=" not in doc:
        fautes.append("preuve du cout Document absente")
    if "clock_gettime(CLOCK_MONOTONIC" not in doc:
        fautes.append("temps Document non mesure")
    appels = re.findall(r"python3 tools/ladybird/(prepare-[\w-]+\.py)", chaine)
    if "prepare-c22-partitions.py" not in appels:
        fautes.append("preparateur non appele par la chaine")
    elif "prepare-echange-processus.py" not in appels or appels.index("prepare-c22-partitions.py") <= appels.index("prepare-echange-processus.py"):
        fautes.append("preparateur mal ordonne")
    return fautes


MUTATIONS = [
    ("PREP", '("layout_heap", 2)', '("layout_heap", 1)'),
    ("PREP", "static thread_local mi_heap_t* s_bouchaud_partitions[4]", "static mi_heap_t* s_bouchaud_partitions[4]"),
    ("PREP", "static thread_local mi_heap_t* {nom}", "static mi_heap_t* {nom}"),
    ("PREP", "s_bouchaud_partitions[index] = heap;", "s_bouchaud_partitions[0] = heap;"),
    ("PREP", "for (auto* heap : s_bouchaud_partitions)", "for (auto* heap : s_fake_partitions)"),
    ("PREP", "mi_heap_collect(heap, true);", "mi_heap_collect(heap, false);"),
    ("PREP", "if (!temoin) {", "if (true) {"),
    ("PREP", "mi_collect(true);", "mi_collect(false);"),
    ("PREP", "ak_kmalloc_collect();", "/* collecte retiree */"),
    ("PREP", "document_destroy collecte duree_us=", "document_destroy mesure_absente="),
    ("CHAINE", 'python3 tools/ladybird/prepare-c22-partitions.py "$SRC"', ""),
]


def main() -> int:
    prep = (RACINE / PREP).read_text(encoding="utf-8")
    chaine = (RACINE / CHAINE).read_text(encoding="utf-8")
    fautes = verifie(prep, chaine)
    if fautes:
        print("C22_PARTITIONS_GARDE_ECHEC", *fautes, sep="\n  ")
        return 1
    for cible, ancien, nouveau in MUTATIONS:
        p, c = prep, chaine
        if cible == "PREP":
            if ancien not in p:
                print(f"C22_PARTITIONS_MUTATION_INAPPLICABLE {ancien!r}")
                return 1
            p = p.replace(ancien, nouveau, 1)
        else:
            if ancien not in c:
                print(f"C22_PARTITIONS_MUTATION_INAPPLICABLE {ancien!r}")
                return 1
            c = c.replace(ancien, nouveau, 1)
        if not verifie(p, c):
            print(f"C22_PARTITIONS_MUTATION_NON_DETECTEE {ancien!r}")
            return 1
    print(f"C22_PARTITIONS_GARDE_OK negatifs={len(MUTATIONS)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
