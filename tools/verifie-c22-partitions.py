#!/usr/bin/env python3
"""Garde C22 (BOUCHAUD_C22_PARTITIONS_COLLECTEES_V1).

Le preparateur doit : enregistrer le fil proprietaire des QUATRE tas
partitionnes, les collecter dans ak_kmalloc_collect() depuis ce fil seulement,
garder le temoin derriere sa variable d'environnement, etre appele par la
chaine. Chaque mutation ci-dessous doit faire echouer la garde.
"""
import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
PREP = "tools/ladybird/prepare-c22-partitions.py"
CHAINE = "tools/ladybird/browser-upstream.sh"


def verifie(prep: str, chaine: str) -> list[str]:
    fautes = []
    for nom, index in (("array_buffer_heap", 0), ("js_object_storage_heap", 1), ("layout_heap", 2), ("painting_heap", 3)):
        if f'("{nom}", {index})' not in prep:
            fautes.append(f"tas {nom} non enregistre (index {index})")
    if "pthread_equal(partition.owner, pthread_self())" not in prep:
        fautes.append("collecte sans controle du fil proprietaire")
    if "mi_heap_collect(heap, true);" not in prep:
        fautes.append("les tas partitionnes ne sont pas collectes")
    if "s_bouchaud_partitions[index].owner = pthread_self();" not in prep:
        fautes.append("proprietaire non retenu a la creation")
    if "heap.store(heap, std::memory_order_release)" not in prep or "load(std::memory_order_acquire)" not in prep:
        fautes.append("publication du tas sans release/acquire")
    if 'getenv(\\"BOUCHAUD_LB_C22_TEMOIN_HISTORIQUE\\")' not in prep or "if (!temoin) {" not in prep:
        fautes.append("temoin absent ou non conditionnel")
    if "    mi_collect(true);\\n" not in prep:
        fautes.append("la collecte du tas par defaut a disparu")
    if re.search(r"mi_collect\(false\)|mi_heap_destroy|mi_heap_delete", prep):
        fautes.append("operation de tas interdite (collecte faible ou destruction)")
    if '"Libraries/LibWeb/DOM/Document.cpp"' not in prep:
        fautes.append("Document::destroy non prepare")
    doc = prep.split('    document = Path(', 1)[1] if '    document = Path(' in prep else ''
    if '"            ak_kmalloc_collect();\\n"' not in doc:
        fautes.append("collecte mimalloc non appelee depuis Document::destroy")
    if 'getenv(\\\"BOUCHAUD_LB_C22_TEMOIN_HISTORIQUE\\\")' not in doc or '"        if (!temoin) {\\n"' not in doc:
        fautes.append("collecte Document::destroy active sous temoin historique")
    if "[LB:C22] document_destroy collecte duree_us=" not in doc or "clock_gettime(CLOCK_MONOTONIC" not in doc:
        fautes.append("point de collecte Document sans preuve de cout")
    if "BOUCHAUD_C22_DOCUMENT_COLLECTE_V2" not in doc:
        fautes.append("repere C22 Document absent")
    appels = re.findall(r"python3 tools/ladybird/(prepare-[\w-]+\.py)", chaine)
    if "prepare-c22-partitions.py" not in appels:
        fautes.append("preparateur non appele par la chaine")
    elif appels.index("prepare-c22-partitions.py") < appels.index("prepare-echange-processus.py"):
        fautes.append("preparateur appele avant prepare-echange-processus.py")
    return fautes


MUTATIONS = [
    ("PREP", '("layout_heap", 2), ', ""),
    ("PREP", '("painting_heap", 3)', '("painting_heap", 2)'),
    ("PREP", "pthread_equal(partition.owner, pthread_self())", "true"),
    ("PREP", "mi_heap_collect(heap, true);", "(void)heap;"),
    ("PREP", "s_bouchaud_partitions[index].owner = pthread_self();", ""),
    ("PREP", "heap.store(heap, std::memory_order_release)", "heap.store(heap, std::memory_order_relaxed)"),
    ("PREP", "if (!temoin) {", "if (true) {"),
    ("PREP", "    mi_collect(true);\\n\",\n    )", "    mi_collect(false);\\n\",\n    )"),
    ("CHAINE", 'python3 tools/ladybird/prepare-c22-partitions.py "$SRC"\n', ""),
    ("PREP", "mi_heap_collect(heap, true);", "mi_heap_destroy(heap);"),
    ("PREP", '"Libraries/LibWeb/DOM/Document.cpp"', '"LibWeb/DOM/Document.cpp"'),
    ("PREP", '"            ak_kmalloc_collect();\\n"', '"            // collecte manquante\\n"'),
    ("PREP", '"        if (!temoin) {\\n"', '"        if (true) {\\n"'),
    ("PREP", 'document_destroy collecte duree_us=', 'document_destroy autre_mesure='),
]


def main() -> int:
    prep = (RACINE / PREP).read_text(encoding="utf-8")
    chaine = (RACINE / CHAINE).read_text(encoding="utf-8")
    fautes = verifie(prep, chaine)
    if fautes:
        print("C22_PARTITIONS_GARDE_ECHEC", *fautes, sep="\n  ")
        return 1
    for cible, avant, apres in MUTATIONS:
        p, c = prep, chaine
        if cible == "PREP":
            if avant not in p:
                print(f"mutation inapplicable : {avant!r}"); return 1
            p = p.replace(avant, apres, 1)
        else:
            if avant not in c:
                print(f"mutation inapplicable : {avant!r}"); return 1
            c = c.replace(avant, apres, 1)
        if not verifie(p, c):
            print(f"mutation non detectee : {avant!r} -> {apres!r}"); return 1
    print(f"C22_PARTITIONS_GARDE_OK negatifs={len(MUTATIONS)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
