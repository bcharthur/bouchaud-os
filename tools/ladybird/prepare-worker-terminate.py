#!/usr/bin/env python3
"""`Worker.terminate()` termine REELLEMENT le worker.

BOUCHAUD_WORKER_TERMINATE_V1 (P5)

Upstream (`Libraries/LibWeb/HTML/Worker.cpp`, epingle cdfe5f8) :

    WebIDL::ExceptionOr<void> Worker::terminate()
    {
        // FIXME: The terminate() method steps are to terminate a worker given this's worker.
        return {};
    }

La batterie de workers du smoke (run 37488566594) l'a mesure : douze
messages recus par la page dans les 600 ms qui suivent `terminate()`, et le
processus WebWorker -- avec son `setInterval` -- vivant jusqu'a la fin de la
session. Chaque worker « termine » continuait de consommer du processeur ; la
molette suivante a mis 22 s a defiler.

Correctif, au plus pres de « terminate a worker » (HTML, workers) :
  * le port exterieur est FERME (detache, desenchevetre, files videes) :
    aucun message ne parvient plus a la page (etape 4) ;
  * l'agent est ferme par le chemin que `finalize()` empruntait deja
    (`close_worker_agent`) : le gestionnaire de processus retire ce
    proprietaire et termine le processus WebWorker, ce qui abandonne le
    script et sa file de taches (etapes 1 a 3) ;
  * idempotent ; `finalize()` ne referme pas un agent deja ferme.

Trois fichiers LibWeb, ancres strictes, fail-closed. Candidat a l'envoi
upstream tel quel.
"""
import sys
from pathlib import Path

MARQUEUR = "BOUCHAUD_WORKER_TERMINATE_V1"


def remplace(chemin: Path, ancre: str, nouveau: str) -> None:
    texte = chemin.read_text(encoding="utf-8")
    if MARQUEUR in texte:
        return
    if texte.count(ancre) != 1:
        raise SystemExit(f"worker terminate : ancre introuvable ou ambigue dans {chemin}")
    chemin.write_text(texte.replace(ancre, nouveau, 1), encoding="utf-8")


MARQUEUR_DOCUMENT = "BOUCHAUD_WORKER_FIN_DOCUMENT_V1"


def remplace_document(chemin: Path, ancre: str, nouveau: str) -> None:
    """Comme `remplace`, sous le second marqueur : ces fichiers portent deja
    le premier, qui ferait tout sauter."""
    texte = chemin.read_text(encoding="utf-8")
    if nouveau in texte:
        return
    if texte.count(ancre) != 1:
        raise SystemExit(f"worker fin de document : ancre introuvable ou ambigue dans {chemin}")
    chemin.write_text(texte.replace(ancre, nouveau, 1), encoding="utf-8")


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: prepare-worker-terminate.py <arbre-ladybird>", file=sys.stderr)
        return 2
    html = Path(sys.argv[1]).resolve() / "Libraries/LibWeb/HTML"

    remplace(
        html / "WorkerAgentParent.h",
        "    static WEB_API void did_close_worker(WorkerAgentOwnerToken);\n",
        "    static WEB_API void did_close_worker(WorkerAgentOwnerToken);\n\n"
        f"    // {MARQUEUR}\n"
        "    // https://html.spec.whatwg.org/multipage/workers.html#terminate-a-worker\n"
        "    void terminate();\n",
    )
    remplace(
        html / "WorkerAgentParent.cpp",
        "void WorkerAgentParent::visit_edges(Cell::Visitor& visitor)\n",
        f"// {MARQUEUR}\n"
        "// https://html.spec.whatwg.org/multipage/workers.html#terminate-a-worker\n"
        "void WorkerAgentParent::terminate()\n"
        "{\n"
        "    // 4. Empty the port message queue of the port that the worker's implicit port is entangled with: close the\n"
        "    //    outside port, so that nothing the worker posted reaches the document any more.\n"
        "    if (m_outside_port)\n"
        "        m_outside_port->close();\n"
        "\n"
        "    worker_agent_parents().remove(m_owner_token);\n"
        "    release_startup_keep_alive();\n"
        "\n"
        "    // 1-3. Set the closing flag, discard queued tasks, abort the running script: the worker lives in its own\n"
        "    //      WebWorker process, and closing its agent ends that process.\n"
        "    if (m_agent_id != 0) {\n"
        "        Bindings::principal_host_defined_page(m_outside_settings->realm()).client().close_worker_agent(m_agent_id, m_owner_token);\n"
        "        m_agent_id = 0;\n"
        "    }\n"
        "}\n\n"
        "void WorkerAgentParent::visit_edges(Cell::Visitor& visitor)\n",
    )
    remplace(
        html / "Worker.cpp",
        "    // FIXME: The terminate() method steps are to terminate a worker given this's worker.\n"
        "    return {};\n",
        f"    // {MARQUEUR}\n"
        "    // The terminate() method steps are to terminate a worker given this's worker.\n"
        "    if (m_agent)\n"
        "        m_agent->terminate();\n"
        "    else if (m_outside_port)\n"
        "        m_outside_port->close();\n"
        "    return {};\n",
    )

    # BOUCHAUD_WORKER_FIN_DOCUMENT_V1 -- un worker dedie meurt avec le
    # document qui l'a cree.
    #
    # Upstream (cdfe5f8) ne termine un worker dedie que lorsque son objet
    # `Worker` est ramasse par le GC (`finalize()` -> close_worker_agent).
    # Quitter la page ne le tue donc pas : banc cycle des workers (run
    # 37622716750) -- 3 WebWorker vivants apres la navigation, jusqu'a la
    # fermeture du navigateur ; endurance -- 27 WebWorker crees, 0 sortis,
    # chacun avec ses minuteries. HTML : un document jete (discarded) est
    # retire de l'ensemble des proprietaires de ses workers, et un worker sans
    # proprietaire est termine. Pour un worker DEDIE, le seul proprietaire est
    # ce document : le terminer quand le document est jete. Un document
    # recuperable (salvageable, gardé dans l'historique) les garde, comme le
    # veut la regle. Les workers partages (plusieurs proprietaires) ne sont
    # pas touches.
    remplace_document(
        html / "WorkerAgentParent.h",
        "    void terminate();\n",
        "    void terminate();\n\n"
        f"    // {MARQUEUR_DOCUMENT}\n"
        "    // Termine les workers DEDIES dont le proprietaire est `document` (document jete).\n"
        "    static WEB_API void terminate_dedicated_workers_of(DOM::Document&);\n",
    )
    remplace_document(
        html / "WorkerAgentParent.cpp",
        "void WorkerAgentParent::visit_edges(Cell::Visitor& visitor)\n",
        f"// {MARQUEUR_DOCUMENT}\n"
        "void WorkerAgentParent::terminate_dedicated_workers_of(DOM::Document& document)\n"
        "{\n"
        "    // `terminate()` retire l'agent de la table : relever d'abord, terminer ensuite.\n"
        "    Vector<GC::Ref<WorkerAgentParent>> owned;\n"
        "    for (auto& entry : worker_agent_parents()) {\n"
        "        auto& agent = entry.value;\n"
        "        if (agent->m_agent_type != AgentType::DedicatedWorker)\n"
        "            continue;\n"
        "        auto* window = window_from_global_object(agent->m_outside_settings->global_object());\n"
        "        if (window && &window->associated_document() == &document)\n"
        "            owned.append(agent);\n"
        "    }\n"
        "    for (auto& agent : owned) {\n"
        "        dbgln(\"[LB] WORKER_FIN_DOCUMENT agent={}\", agent->m_agent_id);\n"
        "        agent->terminate();\n"
        "    }\n"
        "}\n\n"
        "void WorkerAgentParent::visit_edges(Cell::Visitor& visitor)\n",
    )
    dom = Path(sys.argv[1]).resolve() / "Libraries/LibWeb/DOM"
    remplace_document(
        dom / "Document.cpp",
        "#include <LibWeb/HTML/Window.h>\n",
        "#include <LibWeb/HTML/Window.h>\n"
        "#include <LibWeb/HTML/WorkerAgentParent.h>\n",
    )
    remplace_document(
        dom / "Document.cpp",
        "        // 2. Clear window's map of active timers.\n"
        "        window.clear_map_of_active_timers();\n"
        "    }\n",
        "        // 2. Clear window's map of active timers.\n"
        "        window.clear_map_of_active_timers();\n"
        "\n"
        f"        // {MARQUEUR_DOCUMENT} : le document est jete -- ses workers dedies n'ont plus de\n"
        "        // proprietaire (HTML, workers : owner set vide => terminer le worker).\n"
        "        HTML::WorkerAgentParent::terminate_dedicated_workers_of(*this);\n"
        "    }\n",
    )
    print("worker terminate : Worker::terminate() termine l'agent et ferme le port exterieur ; "
          "un document jete termine ses workers dedies")
    return 0


if __name__ == "__main__":
    sys.exit(main())
