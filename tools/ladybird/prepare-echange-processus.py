#!/usr/bin/env python3
"""La page laissee derriere un changement de processus est fermee.

BOUCHAUD_ECHANGE_PROCESSUS_V1

Banc memoire-onglets (run 37746917003, 07880ce6), deux phases identiques de
dix onglets autre site ouverts puis fermes :

    contextes Compositor vivants   3 -> 14 -> 22
    surfaces vivantes              2 -> 26 -> 42
    octets des surfaces            4,6 -> 71 -> 97 Mio
    RSS du WebContent de l'ouvreur +12,8 puis +11,7 Mio par phase

Un contexte et deux surfaces (1100 x 604 x 4 = 2,66 Mo chacune) de plus par
onglet, et qui ne repartent jamais : une fuite, pas un cache.

Un onglet ouvert par `window.open` nait dans le WebContent de son ouvreur,
puis navigue vers l'autre site : l'UI lui donne un NOUVEAU WebContent
(`ViewImplementation::create_new_process_for_cross_site_navigation`). Pour
l'ancien, l'UI ne fait que `unregister_view` -- elle oublie la page, mais ne
dit rien au WebContent qui l'heberge. Si ce WebContent n'a plus d'autre vue,
`close_server_if_unused` le termine et tout part avec lui. S'il en a encore
une -- l'ouvreur, cas de tout `window.open` et de tout `target=_blank` --
la page reste : son document, ses navigables, et dans le Compositor son
contexte et ses surfaces de rendu, pour toute la vie de l'ouvreur.

Correctif : avant de l'oublier, l'UI demande a l'ancien WebContent de FERMER
cette page, par le chemin existant (`WebContentClient::request_close` :
fermeture detachee, le processus est garde vivant jusqu'a l'accuse
`did_close_browsing_context`, qui ne trouve plus de vue et ne fait que
liberer). Cote WebContent, `close_top_level_traversable` decharge le document
et detruit les navigables, donc leurs contextes Compositor.

Limite connue : une page qui a un gestionnaire `beforeunload` ACTIF, avec
activation utilisateur, demanderait une confirmation que plus aucune vue ne
peut afficher ; elle resterait ouverte (pas de blocage ailleurs).

Ancres strictes, fail-closed, idempotent.
"""
import sys
from pathlib import Path

MARQUEUR = "BOUCHAUD_ECHANGE_PROCESSUS_V1"


def remplace(chemin: Path, ancre: str, nouveau: str) -> None:
    texte = chemin.read_text(encoding="utf-8")
    if nouveau in texte:
        return
    if texte.count(ancre) != 1:
        raise SystemExit(f"echange de processus : ancre introuvable ou ambigue dans {chemin} :\n{ancre}")
    chemin.write_text(texte.replace(ancre, nouveau, 1), encoding="utf-8")


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: prepare-echange-processus.py <arbre-ladybird>", file=sys.stderr)
        return 2
    racine = Path(sys.argv[1]).resolve()
    remplace(
        racine / "Libraries/LibWebView/ViewImplementation.cpp",
        "    if (m_client_state.client) {\n"
        "        m_client_state.client->async_notify_webdriver_of_window_replacement(m_client_state.page_index);\n"
        "        m_client_state.client->unregister_view(m_client_state.page_index);\n"
        "    }\n",
        "    if (m_client_state.client) {\n"
        "        m_client_state.client->async_notify_webdriver_of_window_replacement(m_client_state.page_index);\n"
        f"        // {MARQUEUR} : the old process may still host other views (the opener of a window.open). Close\n"
        "        // this page there, or its document, navigables and Compositor surfaces live as long as that process.\n"
        "        dbgln(\"[LB] PROCESS_SWAP_CLOSE_OLD_PAGE pid={} page={}\", m_client_state.client->pid(), m_client_state.page_index);\n"
        "        m_client_state.client->request_close(m_client_state.page_index);\n"
        "        m_client_state.client->unregister_view(m_client_state.page_index);\n"
        "    }\n",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
