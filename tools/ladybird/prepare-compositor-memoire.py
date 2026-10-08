#!/usr/bin/env python3
"""Ce que le Compositor garde en memoire, publie a chaque changement.

BOUCHAUD_LB_MEM_V1

Endurance 37667817559 (e6799c24) : RSS du Compositor 38 -> 102 Mio en 10 min
sous TCG, 30 -> 81 Mio en 5 min sous KVM. Fuite ou cache borne ? Les
compteurs existants ne le disent pas : `CONTEXT_DESTROY` n'est journalise que
pour un `destroy_context` explicite, pas pour la destruction en bloc des
contextes d'un WebContent qui meurt (`PEER_CLOSE`) -- 67 creations contre
51 destructions ne prouvent donc rien.

Une ligne a chaque creation ou destruction de contexte, quel qu'en soit le
chemin (CompositorState, par ou passent les deux) :

    [LB:MEM] ev=context_create|context_destroy ctx= contexts_live=
             backing_stores_live= backing_store_octets=
             skia_ressources_octets= skia_ressources_limite= skia_polices_octets=

`backing_store_*` : surfaces de rendu allouees par TOUS les contextes vivants
(BackingStoreManager), en octets (largeur x hauteur x 4). `skia_*` : caches
globaux de Skia -- bornes par construction ; leur limite est publiee pour
que la borne se lise dans le journal, pas dans la doc. Les connexions
vivantes sont deja publiees par CONNECTION_CREATE total= / CONNECTION_REMOVE
restantes= (prepare-compositor-lien.py).

Rien ne change de comportement : des lectures et une ligne de journal.
Ancres strictes, fail-closed, idempotent.
"""
import sys
from pathlib import Path

MARQUEUR = "BOUCHAUD_LB_MEM_V1"


def remplace(chemin: Path, ancre: str, nouveau: str) -> None:
    texte = chemin.read_text(encoding="utf-8")
    if nouveau in texte:
        return
    if texte.count(ancre) != 1:
        raise SystemExit(f"compositor memoire : ancre introuvable ou ambigue dans {chemin} :\n{ancre}")
    chemin.write_text(texte.replace(ancre, nouveau, 1), encoding="utf-8")


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: prepare-compositor-memoire.py <arbre-ladybird>", file=sys.stderr)
        return 2
    racine = Path(sys.argv[1]).resolve()
    comp = racine / "Services/Compositor"

    # --- BackingStoreManager : ce qu'il tient ------------------------------
    remplace(
        comp / "BackingStoreManager.h",
        "    RefPtr<Gfx::PaintingSurface> latest_rendered_surface() const;\n\nprivate:\n",
        "    RefPtr<Gfx::PaintingSurface> latest_rendered_surface() const;\n"
        "\n"
        f"    // {MARQUEUR}\n"
        "    size_t bouchaud_surfaces_vivantes() const\n"
        "    {\n"
        "        size_t n = 0;\n"
        "        for (auto const& store : m_backing_stores) {\n"
        "            if (store.surface)\n"
        "                ++n;\n"
        "        }\n"
        "        return n;\n"
        "    }\n"
        "    size_t bouchaud_octets() const\n"
        "    {\n"
        "        size_t n = 0;\n"
        "        for (auto const& store : m_backing_stores) {\n"
        "            if (store.surface) {\n"
        "                auto taille = store.surface->size();\n"
        "                n += static_cast<size_t>(taille.width()) * static_cast<size_t>(taille.height()) * 4;\n"
        "            }\n"
        "        }\n"
        "        return n;\n"
        "    }\n"
        "\n"
        "private:\n",
    )

    # --- ContextState : l'exposer en lecture --------------------------------
    remplace(
        comp / "ContextState.h",
        "    RefPtr<Gfx::PaintingSurface> latest_rendered_surface() const { return m_latest_rendered_surface; }\n",
        "    RefPtr<Gfx::PaintingSurface> latest_rendered_surface() const { return m_latest_rendered_surface; }\n"
        f"    // {MARQUEUR}\n"
        "    BackingStoreManager const& bouchaud_backing_stores() const { return m_backing_store_manager; }\n",
    )

    # --- CompositorState : la ligne, a chaque creation et destruction -------
    remplace(
        comp / "CompositorState.h",
        "    void destroy_contexts_for_web_content_client(CompositorStateWebContentClient&);\n",
        "    void destroy_contexts_for_web_content_client(CompositorStateWebContentClient&);\n"
        f"    // {MARQUEUR}\n"
        "    void bouchaud_publie_memoire(StringView evenement, Web::Compositor::CompositorContextId) const;\n",
    )
    cs = comp / "CompositorState.cpp"
    remplace(
        cs,
        "#include <LibCore/Timer.h>\n",
        "#include <LibCore/Timer.h>\n"
        f"// {MARQUEUR}\n"
        "#include <core/SkGraphics.h>\n",
    )
    remplace(
        cs,
        "    resize_backing_stores_if_needed(context_id, context);\n}\n",
        "    resize_backing_stores_if_needed(context_id, context);\n"
        f"    bouchaud_publie_memoire(\"context_create\"sv, context_id); // {MARQUEUR}\n"
        "}\n"
        "\n"
        f"// {MARQUEUR}\n"
        "void CompositorState::bouchaud_publie_memoire(StringView evenement, Web::Compositor::CompositorContextId context_id) const\n"
        "{\n"
        "    size_t surfaces = 0;\n"
        "    size_t octets = 0;\n"
        "    for (auto const& entree : m_contexts) {\n"
        "        surfaces += entree.value->bouchaud_backing_stores().bouchaud_surfaces_vivantes();\n"
        "        octets += entree.value->bouchaud_backing_stores().bouchaud_octets();\n"
        "    }\n"
        "    dbgln(\"[LB:MEM] ev={} ctx={} contexts_live={} backing_stores_live={} backing_store_octets={} \"\n"
        "          \"skia_ressources_octets={} skia_ressources_limite={} skia_polices_octets={}\",\n"
        "        evenement, context_id.value(), m_contexts.size(), surfaces, octets,\n"
        "        SkGraphics::GetResourceCacheTotalBytesUsed(), SkGraphics::GetResourceCacheTotalByteLimit(),\n"
        "        SkGraphics::GetFontCacheUsed());\n"
        "}\n",
    )
    remplace(
        cs,
        "    m_contexts.remove(context_id);\n    update_video_sink_ticking_states();\n}\n",
        "    m_contexts.remove(context_id);\n"
        "    update_video_sink_ticking_states();\n"
        f"    bouchaud_publie_memoire(\"context_destroy\"sv, context_id); // {MARQUEUR}\n"
        "}\n",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
