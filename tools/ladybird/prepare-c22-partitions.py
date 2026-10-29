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
        "#    include <atomic>   // BOUCHAUD_C22_PARTITIONS_COLLECTEES_V1\n"
        "#    include <cstdio>\n"
        "#    include <cstdlib>\n"
        "#    include <pthread.h>\n"
        "#endif\n",
    )
    remplace(
        kmalloc,
        "static thread_local mi_heap_t* s_string_heap = nullptr;\n",
        "static thread_local mi_heap_t* s_string_heap = nullptr;\n"
        "\n"
        "// BOUCHAUD_C22_PARTITIONS_COLLECTEES_V1 : les tas partages entre appels\n"
        "// (ArrayBuffer, JSObjectStorage, Layout, Painting) retiennent leur fil\n"
        "// proprietaire, pour que ak_kmalloc_collect() les collecte depuis CE fil.\n"
        "struct BouchaudPartitionHeap {\n"
        "    std::atomic<mi_heap_t*> heap { nullptr };\n"
        "    pthread_t owner {};\n"
        "};\n"
        "static BouchaudPartitionHeap s_bouchaud_partitions[4];\n"
        "\n"
        "static mi_heap_t* bouchaud_partition_new(size_t index)\n"
        "{\n"
        "    auto* heap = mi_heap_new();\n"
        "    s_bouchaud_partitions[index].owner = pthread_self();\n"
        "    s_bouchaud_partitions[index].heap.store(heap, std::memory_order_release);\n"
        "    return heap;\n"
        "}\n",
    )
    for nom, index in (("array_buffer_heap", 0), ("js_object_storage_heap", 1), ("layout_heap", 2), ("painting_heap", 3)):
        remplace(
            kmalloc,
            f"        static mi_heap_t* {nom} = mi_heap_new();\n",
            f"        static mi_heap_t* {nom} = bouchaud_partition_new({index});\n",
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
        "        for (auto& partition : s_bouchaud_partitions) {\n"
        "            auto* heap = partition.heap.load(std::memory_order_acquire);\n"
        "            if (heap && pthread_equal(partition.owner, pthread_self())) {\n"
        "                mi_heap_collect(heap, true);\n"
        "                ++collectes;\n"
        "            }\n"
        "        }\n"
        "    }\n"
        "    if (preuve)\n"
        "        fprintf(stderr, \"[LB:C22] collecte partitions=%d temoin=%d\\n\", collectes, temoin ? 1 : 0);\n"
        "\n"
        "    mi_collect(true);\n",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
