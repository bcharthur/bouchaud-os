#!/usr/bin/env python3
"""C22 -- les tas mimalloc partitionnes de WebContent rendent leurs pages.

BOUCHAUD_C22_PARTITIONS_COLLECTEES_V1

    prepare-c22-partitions.py <arbre-ladybird>

Defaut (runs 40832423213, 40851662593, 40860559456) : apres deux phases
identiques d'onglets ouverts puis fermes, le RSS de WebContent garde ~5 Mio
anonymes dans une repetition sur deux. Le noyau situe ces pages dans le
segment mimalloc du fil principal (mmap 32 Mio, pile `_mi_segment_page_alloc`)
-- ni bitmap partage, ni fichier, ni tas LibGC.

Cause : `AK/kmalloc.cpp` sert les noeuds de layout, les paintables et les
stacking contexts depuis des tas mimalloc dedies (`HeapPartition::Layout`,
`Painting`, ainsi que `ArrayBuffer` et `JSObjectStorage`), crees par
`mi_heap_new()`. `ak_kmalloc_collect()` -- appele une fois par balayage GC
complet (BOUCHAUD_P13_MIMALLOC_POST_SWEEP_V1) -- ne collecte que le tas par
defaut et celui des chaines : les pages videes dans les autres tas, quand les
documents des onglets fermes sont detruits par ce balayage, restent engagees
jusqu'a ce qu'une allocation ulterieure DANS CE TAS les recycle. D'ou la
bimodalite : selon que la phase suivante realloue ou non dans ces tas.

Correctif : chaque tas partitionne retient son fil proprietaire ;
`ak_kmalloc_collect()` collecte aussi ceux qui appartiennent au fil appelant
(un tas mimalloc ne se collecte que depuis son fil). `mi_heap_collect` ne
rend que des pages VIDES : aucune allocation vivante n'est touchee, aucun GC
n'est declenche, le rythme reste celui des balayages deja en place.

Temoin : BOUCHAUD_LB_C22_TEMOIN_HISTORIQUE=1 (lu une fois) retablit le
comportement d'origine dans le MEME binaire -- banc memoire seulement.
Avec BOUCHAUD_LB_MEMORY_PROOF, chaque collecte publie
`[LB:C22] collecte partitions=N temoin=0|1`.
"""
import sys
from pathlib import Path


def remplace(chemin: Path, ancre: str, nouveau: str) -> None:
    texte = chemin.read_text(encoding="utf-8")
    if nouveau in texte:
        return
    if texte.count(ancre) != 1:
        raise SystemExit(f"c22 partitions : ancre introuvable ou ambigue dans {chemin} :\n{ancre}")
    chemin.write_text(texte.replace(ancre, nouveau, 1), encoding="utf-8")


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: prepare-c22-partitions.py <arbre-ladybird>", file=sys.stderr)
        return 2
    kmalloc = Path(sys.argv[1]).resolve() / "AK/kmalloc.cpp"
    remplace(
        kmalloc,
        "#    include <mimalloc.h>\n#endif\n",
        "#    include <mimalloc.h>\n"
        "#    include <cstdio>  // BOUCHAUD_C22_PARTITIONS_PAR_FIL_V2\n"
        "#    include <cstdlib>\n"
        "#endif\n",
    )
    remplace(
        kmalloc,
        "static thread_local mi_heap_t* s_string_heap = nullptr;\n",
        "static thread_local mi_heap_t* s_string_heap = nullptr;\n"
        "\n"
        "// BOUCHAUD_C22_PARTITIONS_PAR_FIL_V2: mimalloc heaps belong to\n"
        "// their creating thread. Do not publish one heap globally for\n"
        "// allocations from multiple threads. Keep one heap per partition\n"
        "// PER THREAD, and collect it only from that thread.\n"
        "static thread_local mi_heap_t* s_bouchaud_partitions[4] = { nullptr, nullptr, nullptr, nullptr };\n"
        "\n"
        "static mi_heap_t* bouchaud_partition_new(size_t index)\n"
        "{\n"
        "    auto* heap = mi_heap_new();\n"
        "    s_bouchaud_partitions[index] = heap;\n"
        "    return heap;\n"
        "}\n",
    )
    for nom, index in (("array_buffer_heap", 0), ("js_object_storage_heap", 1), ("layout_heap", 2), ("painting_heap", 3)):
        remplace(
            kmalloc,
            f"        static mi_heap_t* {nom} = mi_heap_new();\n",
            f"        static thread_local mi_heap_t* {nom} = bouchaud_partition_new({index});\n",
        )
    remplace(
        kmalloc,
        "    if (s_string_heap)\n"
        "        mi_heap_collect(s_string_heap, true);\n"
        "\n"
        "    mi_collect(true);\n",
        "    if (s_string_heap)\n"
        "        mi_heap_collect(s_string_heap, true);\n"
        "\n"
        "    // BOUCHAUD_C22_PARTITIONS_COLLECTEES_V1 : les pages videes dans les tas\n"
        "    // partitionnes (layout et paintables des documents detruits) sont\n"
        "    // rendues ici, depuis le fil qui possede chaque tas. Seules des pages\n"
        "    // VIDES sont rendues.\n"
        "    static bool const temoin = getenv(\"BOUCHAUD_LB_C22_TEMOIN_HISTORIQUE\") != nullptr;\n"
        "    static bool const preuve = getenv(\"BOUCHAUD_LB_MEMORY_PROOF\") != nullptr;\n"
        "    int collectes = 0;\n"
        "    if (!temoin) {\n"
        "        for (auto* heap : s_bouchaud_partitions) {\n"
        "            if (!heap)\n"
        "                continue;\n"
        "            mi_heap_collect(heap, true);\n"
        "            ++collectes;\n"
        "        }\n"
        "    }\n"
        "    if (preuve)\n"
        "        fprintf(stderr, \"[LB:C22] collecte partitions=%d temoin=%d\\n\", collectes, temoin ? 1 : 0);\n"
        "\n"
        "    mi_collect(true);\n",
    )
    # BOUCHAUD_C22_DOCUMENT_COLLECTE_V2 : les noeuds Layout/Painting
    # sont detruits par Document::destroy() (comptage de references), sans
    # attendre un cycle GC. Collecter les heaps sur ce meme fil apres la
    # destruction (aucun GC JS force, aucune allocation vivante liberee).
    document = Path(sys.argv[1]).resolve() / "Libraries/LibWeb/DOM/Document.cpp"
    remplace(
        document,
        "#include <AK/Time.h>\n",
        "#include <AK/Time.h>\n"
        "#include <AK/kmalloc.h> // BOUCHAUD_C22_DOCUMENT_COLLECTE_V2\n"
        "#include <cstdio>\n"
        "#include <cstdlib>\n"
        "#include <time.h>\n",
    )
    remplace(
        document,
        "    // FIXME: 11. For each workletGlobalScope in document's worklet global scopes, terminate workletGlobalScope.\n}\n",
        "    // FIXME: 11. For each workletGlobalScope in document's worklet global scopes, terminate workletGlobalScope.\n"
        "\n"
        "    // BOUCHAUD_C22_DOCUMENT_COLLECTE_V2 : les objets Layout/Painting\n"
        "    // liberes par le demontage doivent rendre leurs pages maintenant,\n"
        "    // sans attendre un balayage GC qui peut arriver apres le repere M2.\n"
        "    // Collecte mimalloc uniquement, depuis le fil proprietaire.\n"
        "    // Temoin historique : aucune collecte ajoutee dans ce Document.\n"
        "    {\n"
        "        static bool const temoin = getenv(\"BOUCHAUD_LB_C22_TEMOIN_HISTORIQUE\") != nullptr;\n"
        "        static bool const preuve = getenv(\"BOUCHAUD_LB_MEMORY_PROOF\") != nullptr;\n"
        "        if (!temoin) {\n"
        "            struct timespec debut {}, fin {};\n"
        "            if (preuve)\n"
        "                clock_gettime(CLOCK_MONOTONIC, &debut);\n"
        "            ak_kmalloc_collect();\n"
        "            if (preuve) {\n"
        "                clock_gettime(CLOCK_MONOTONIC, &fin);\n"
        "                long duree_us = (fin.tv_sec - debut.tv_sec) * 1000000L + (fin.tv_nsec - debut.tv_nsec) / 1000;\n"
        "                fprintf(stderr, \"[LB:C22] document_destroy collecte duree_us=%ld\\n\", duree_us);\n"
        "            }\n"
        "        }\n"
        "    }\n"
        "}\n",
    )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
