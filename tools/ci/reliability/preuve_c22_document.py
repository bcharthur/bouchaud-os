#!/usr/bin/env python3
"""Preuve rapide : le crochet Document C22 respecte le temoin historique."""
import os
from pathlib import Path
import subprocess
import tempfile

RACINE = Path(__file__).resolve().parents[3]
PREPARATEUR = RACINE / "tools/ladybird/prepare-c22-partitions.py"

def executer(*args, env=None):
    return subprocess.run(args, check=True, capture_output=True, text=True, env=env)

with tempfile.TemporaryDirectory(prefix="bouchaud-c22-doc-") as tmp:
    p = Path(tmp)
    km = p / "AK/kmalloc.cpp"
    doc = p / "Libraries/LibWeb/DOM/Document.cpp"
    km.parent.mkdir(parents=True)
    doc.parent.mkdir(parents=True)
    km.write_text(
        "#    include <mimalloc.h>\n#endif\n"
        "static thread_local mi_heap_t* s_string_heap = nullptr;\n"
        "        static mi_heap_t* array_buffer_heap = mi_heap_new();\n"
        "        static mi_heap_t* js_object_storage_heap = mi_heap_new();\n"
        "        static mi_heap_t* layout_heap = mi_heap_new();\n"
        "        static mi_heap_t* painting_heap = mi_heap_new();\n"
        "    if (s_string_heap)\n"
        "        mi_heap_collect(s_string_heap, true);\n"
        "\n"
        "    mi_collect(true);\n", encoding="utf8"
    )
    doc.write_text(
        "#include <AK/Time.h>\n"
        "void destroy() {\n"
        "    // FIXME: 11. For each workletGlobalScope in document's worklet global scopes, terminate workletGlobalScope.\n"
        "}\n", encoding="utf8"
    )
    executer("python3", str(PREPARATEUR), str(p))
    premier = doc.read_text()
    executer("python3", str(PREPARATEUR), str(p))
    assert doc.read_text() == premier, "preparateur non idempotent"
    marqueur = "    // BOUCHAUD_C22_DOCUMENT_COLLECTE_V2 : les objets Layout/Painting"
    assert premier.count(marqueur) == 1, "marqueur unique absent"
    bloc = premier.split(marqueur, 1)[1].split("\n}\n", 1)[0]
    code = (
        "#include <cstdio>\n#include <cstdlib>\n#include <time.h>\n"
        "static int appels = 0;\n"
        "void ak_kmalloc_collect() { ++appels; }\n"
        "void destroy() {\n" + marqueur + bloc + "\n}\n"
        "int main() { destroy(); const bool temoin = getenv(\"BOUCHAUD_LB_C22_TEMOIN_HISTORIQUE\") != nullptr;"
        "if (appels != (temoin ? 0 : 1)) return 10;"
        "destroy(); if (appels != (temoin ? 0 : 2)) return 11;"
        "return 0; }\n"
    )
    source = p / "doc.cpp"
    output = p / "doc"
    source.write_text(code, encoding="utf8")
    executer("g++", "-std=c++23", "-O2", "-Wall", "-Wextra", "-Werror", str(source), "-o", str(output))
    temoin = dict(os.environ, BOUCHAUD_LB_C22_TEMOIN_HISTORIQUE="1", BOUCHAUD_LB_MEMORY_PROOF="1")
    fix = dict(os.environ, BOUCHAUD_LB_MEMORY_PROOF="1")
    fix.pop("BOUCHAUD_LB_C22_TEMOIN_HISTORIQUE", None)
    a = executer(str(output), env=temoin)
    b = executer(str(output), env=fix)
    assert "document_destroy collecte" not in a.stderr, "temoin contamine"
    assert b.stderr.count("[LB:C22] document_destroy collecte duree_us=") == 2, "preuve/cout absent"
    print("C22_DOCUMENT_TEMOIN_OK temoin=0_collections correctif=2_collections cout=present")
