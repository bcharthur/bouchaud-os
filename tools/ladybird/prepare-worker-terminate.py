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
    print("worker terminate : Worker::terminate() termine l'agent et ferme le port exterieur")
    return 0


if __name__ == "__main__":
    sys.exit(main())
