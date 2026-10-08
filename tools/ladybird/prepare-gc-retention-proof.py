#!/usr/bin/env python3
"""Ajoute une preuve ciblee des chemins GC qui gardent les anciens PageClient.

BOUCHAUD_P13_GC_RETENTION_V1

Ce preparateur s'applique APRES prepare-echange-processus.py. Il ne change
aucune politique GC et ne force aucune collection. Il appelle dump_graph()
uniquement quand le WebContent ouvreur a detache exactement 10 puis 20 pages,
puis imprime les plus courts chemins depuis les racines GC vers les PageClient
encore vivants.

Le but est d'attribuer la retention sans changer le comportement produit.
Ancres strictes, fail-closed, idempotent.
"""
from pathlib import Path
import sys

MARKER = "BOUCHAUD_P13_GC_RETENTION_V1"


def replace_once(path: Path, old: str, new: str, label: str) -> None:
    text = path.read_text(encoding="utf-8")
    if new in text:
        return
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"gc-retention: {label}: ancre attendue 1 fois, trouvee {count} dans {path}")
    path.write_text(text.replace(old, new, 1), encoding="utf-8")


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: prepare-gc-retention-proof.py <arbre-ladybird>", file=sys.stderr)
        return 2

    root = Path(sys.argv[1]).resolve()
    cpp = root / "Services/WebContent/PageHost.cpp"
    if not cpp.is_file():
        raise SystemExit(f"gc-retention: PageHost.cpp absent: {cpp}")

    # Le preparateur precedent doit deja avoir pose la preuve PageHost/PageClient.
    text = cpp.read_text(encoding="utf-8")
    required = [
        "BOUCHAUD_PAGES_MEMOIRE_V1",
        "s_bouchaud_pages_created",
        "s_bouchaud_pages_detached",
        "s_bouchaud_pages_finalized",
        "[LB:PAGE_STATE]",
    ]
    missing = [m for m in required if m not in text]
    if missing:
        raise SystemExit(f"gc-retention: preparation memoire precedente absente: {missing}")
    if MARKER in text:
        return 0

    replace_once(
        cpp,
        "#include <WebContent/PageHost.h>\n"
        "#include <LibCore/System.h>\n"
        "#include <stdlib.h>\n",
        "#include <WebContent/PageHost.h>\n"
        "#include <AK/HashMap.h>\n"
        "#include <AK/JsonArray.h>\n"
        "#include <AK/JsonObject.h>\n"
        "#include <AK/JsonValue.h>\n"
        "#include <AK/StringBuilder.h>\n"
        "#include <AK/Vector.h>\n"
        "#include <LibCore/System.h>\n"
        "#include <LibGC/Heap.h>\n"
        "#include <LibWeb/Bindings/MainThreadVM.h>\n"
        "#include <stdlib.h>\n",
        "includes",
    )

    anchor = (
        "void bouchaud_note_page_finalisee()\n"
        "{\n"
        "    ++s_bouchaud_pages_finalized;\n"
        "}\n"
    )

    helper = anchor + r'''

// BOUCHAUD_P13_GC_RETENTION_V1
//
// dump_graph() ne collecte rien : il draine seulement un sweep incremental
// deja en cours, rassemble les racines et decrit le graphe vivant. Cette
// preuve est volontairement executee hors remove_page(), depuis le timer du
// PageHost, donc apres deroulement de la pile de fermeture de l'onglet.
struct BouchaudGcNode {
    String class_name;
    Optional<String> root;
    Optional<u64> stack_frame_index;
    Vector<String> edges;
};

static u64 s_bouchaud_last_gc_retention_detached = 0;

static void bouchaud_trace_pageclient_retention(size_t active_pages, u64 detached)
{
    auto graph = Web::Bindings::main_thread_vm().heap().dump_graph();
    Vector<String> stack_frame_labels;
    if (auto frames = graph.get_array("stack_frames"sv); frames.has_value()) {
        for (auto const& frame : frames->values()) {
            if (!frame.is_object()) {
                stack_frame_labels.append(String {});
                continue;
            }
            if (auto label = frame.as_object().get_string("label"sv); label.has_value())
                stack_frame_labels.append(*label);
            else
                stack_frame_labels.append(String {});
        }
    }

    HashMap<String, BouchaudGcNode> nodes;

    graph.for_each_member([&](String const& address, JsonValue const& value) {
        if (!value.is_object())
            return;
        auto const& object = value.as_object();
        auto class_name = object.get_string("class_name"sv);
        if (!class_name.has_value())
            return;

        BouchaudGcNode node;
        node.class_name = *class_name;
        if (auto root = object.get_string("root"sv); root.has_value())
            node.root = *root;
        node.stack_frame_index = object.get_u64("stack_frame_index"sv);
        if (auto edges = object.get_array("edges"sv); edges.has_value()) {
            for (auto const& edge : edges->values()) {
                if (edge.is_string())
                    node.edges.append(edge.as_string());
            }
        }
        nodes.set(address, move(node));
    });

    // BFS depuis toutes les racines. parent[root] == root rend la
    // reconstruction bornee et evite un Optional dans la table.
    HashMap<String, String> parent;
    HashMap<String, String> root_label;
    HashMap<String, i64> root_frame;
    Vector<String> queue;
    for (auto const& [address, node] : nodes) {
        if (!node.root.has_value())
            continue;
        parent.set(address, address);
        root_label.set(address, *node.root);
        root_frame.set(address, node.stack_frame_index.has_value() ? static_cast<i64>(node.stack_frame_index.value()) : -1);
        queue.append(address);
    }

    size_t cursor = 0;
    while (cursor < queue.size()) {
        auto address = queue[cursor++];
        auto node = nodes.get(address);
        if (!node.has_value())
            continue;
        auto label = root_label.get(address);
        auto frame = root_frame.get(address);
        if (!label.has_value() || !frame.has_value())
            continue;
        for (auto const& edge : node->edges) {
            if (!nodes.contains(edge) || parent.contains(edge))
                continue;
            parent.set(edge, address);
            root_label.set(edge, *label);
            root_frame.set(edge, *frame);
            queue.append(edge);
        }
    }

    size_t pageclients = 0;
    size_t paths = 0;
    size_t path_limit = 32;
    for (auto const& [address, node] : nodes) {
        if (!node.class_name.contains("PageClient"sv))
            continue;
        ++pageclients;
        if (paths >= path_limit)
            continue;

        auto label = root_label.get(address);
        auto frame = root_frame.get(address);
        auto parent_entry = parent.get(address);
        if (!label.has_value() || !frame.has_value() || !parent_entry.has_value()) {
            dbgln("[LB:GC_PATH] pid={} detached={} target={} root=NO_PATH frame=-1 frame_label= depth=0 path={} END",
                Core::System::getpid(), detached, address, node.class_name);
            ++paths;
            continue;
        }

        Vector<String> classes;
        String current = address;
        for (size_t depth = 0; depth < 64; ++depth) {
            auto current_node = nodes.get(current);
            if (!current_node.has_value())
                break;
            classes.append(current_node->class_name);
            auto current_parent = parent.get(current);
            if (!current_parent.has_value() || *current_parent == current)
                break;
            current = *current_parent;
        }

        StringBuilder path;
        for (size_t i = classes.size(); i > 0; --i) {
            if (i != classes.size())
                path.append('>');
            path.append(classes[i - 1]);
        }
        auto path_string = MUST(path.to_string());
        String frame_label;
        if (*frame >= 0) {
            auto frame_index = static_cast<size_t>(*frame);
            if (frame_index < stack_frame_labels.size())
                frame_label = stack_frame_labels[frame_index];
        }
        dbgln("[LB:GC_PATH] pid={} detached={} target={} root={} frame={} frame_label={} depth={} path={} END",
            Core::System::getpid(), detached, address, *label, *frame, frame_label, classes.size(), path_string);
        ++paths;
    }

    auto retained = pageclients > active_pages ? pageclients - active_pages : 0;
    dbgln("[LB:GC_RETENTION] pid={} detached={} active={} pageclients={} retained={} paths={} END",
        Core::System::getpid(), detached, active_pages, pageclients, retained, paths);
}
'''

    replace_once(cpp, anchor, helper, "helper")

    timer_anchor = (
        "                s_bouchaud_pages_detached,\n"
        "                s_bouchaud_pages_finalized);\n"
        "        });\n"
    )
    timer_new = (
        "                s_bouchaud_pages_detached,\n"
        "                s_bouchaud_pages_finalized);\n"
        "            if (getenv(\"BOUCHAUD_GC_RETENTION_PROOF\")\n"
        "                && (s_bouchaud_pages_detached == 10 || s_bouchaud_pages_detached == 20)\n"
        "                && s_bouchaud_pages_detached != s_bouchaud_last_gc_retention_detached) {\n"
        "                s_bouchaud_last_gc_retention_detached = s_bouchaud_pages_detached;\n"
        "                bouchaud_trace_pageclient_retention(m_pages.size(), s_bouchaud_pages_detached);\n"
        "            }\n"
        "        });\n"
    )
    replace_once(cpp, timer_anchor, timer_new, "declenchement")

    if MARKER not in cpp.read_text(encoding="utf-8"):
        raise SystemExit("gc-retention: marqueur final absent")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
