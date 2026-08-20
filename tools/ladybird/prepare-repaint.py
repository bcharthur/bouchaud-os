#!/usr/bin/env python3
"""Repeindre quand la page a change, et seulement la ou elle s'affiche.

## Le defaut

Le navigateur tenait 90 a 98 % du seul cœur de Bouchaud, en produisant environ
deux trames par seconde, y compris quand rien ne bougeait :

    M8_CPU_SCREENSHOT_STAGE layout begin
    M8_CPU_SCREENSHOT_STAGE layout ok
    M8_CPU_SCREENSHOT_STAGE display-list ok
    M8_CPU_SCREENSHOT_STAGE replay begin
    M8_CPU_SCREENSHOT_RENDERED
    (et ainsi de suite, indefiniment)

Deux causes, independantes.

### 1. Un minuteur demandait les trames a la place du moteur

`BouchaudChrome::tick()` s'execute toutes les 16 ms et, pendant un chargement,
reclamait une trame a chaque tic. Or reclamer une trame passe par

    LocalTraversableNavigable::queue_screenshot_task()
        m_screenshot_tasks.enqueue(...);
        set_needs_repaint();          // <- ici
        page().client().request_frame();

`set_needs_repaint()` est precisement le drapeau que Ladybird consulte a
l'etape 22 de « update the rendering » pour decider s'il faut repeindre :

    if (!navigable || !navigable->needs_repaint())
        continue;

Le minuteur le remettait donc a vrai soixante fois par seconde. Le moteur
possedait deja un modele d'invalidation complet, et nous le rendions inoperant
en repondant « oui » a sa question avant qu'il ne la pose.

La reponse juste est de laisser LibWeb decider. `request_frame()` est appele
depuis `Document`, `StyleComputer`, `Window`, `ResizeObserver`,
`IntersectionObserver` — partout ou quelque chose a reellement change. La
capture s'enfile donc **dans** l'etape de rendu, sous la garde
`needs_repaint()`, et non plus depuis un minuteur exterieur.

### 2. On allouait le document entier pour n'en peindre que la fenetre

La branche « page complete » de `process_screenshot_requests()` dimensionne son
bitmap sur `scrollable_overflow_rect()` — toute la hauteur du document. Pour la
page d'accueil de Wikipedia, plusieurs mebioctets alloues **et effaces** a
chaque trame.

Mais la liste d'affichage, elle, est enregistree en coordonnees de **fenetre** :

    auto viewport_rect = page().css_to_device_rect(this->viewport_rect());
    Gfx::IntRect bitmap_rect { {}, viewport_rect.size().to_type<int>() };
                                                    -- Document::record_display_list

et la presentation Bouchaud ne copie de toute facon que
`min(bitmap, surface)`. Ces mebioctets ne portaient donc aucun pixel visible.

Le bitmap fait desormais la taille de la fenetre d'affichage. Aucun pixel
affiche ne change — la comparaison de capture de M8 et M9 lit exactement les
memes octets — mais l'allocation et l'effacement disparaissent.

## Ce que cela ne change pas

Les scenarios finis M8 et M9 capturent toujours **une** trame, a l'endroit
qu'ils choisissent, et se terminent dessus. L'enfilage automatique de l'etape 1
n'est actif que pour le navigateur interactif (`BOUCHAUD_M11`) : un test
deterministe ne veut pas d'une trame supplementaire decidee par le moteur.
"""
from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-repaint.py <ladybird-worktree>")

root = Path(sys.argv[1])


def substitute(path: Path, old: str, new: str, label: str) -> None:
    data = path.read_text()
    if new in data:
        return
    if old not in data:
        raise SystemExit(f"repaint : ancre introuvable ({label}) dans {path}")
    path.write_text(data.replace(old, new, 1))


# ---------------------------------------------------------------------------
# 1. La capture fait la taille de la fenetre d'affichage, pas du document.
# ---------------------------------------------------------------------------

navigable_cpp = root / "Libraries/LibWeb/HTML/LocalTraversableNavigable.cpp"
substitute(
    navigable_cpp,
    """            active_document()->update_layout(DOM::UpdateLayoutReason::ProcessScreenshot);
            auto scrollable_overflow_rect = active_document()->layout_node()->paintable_box()->scrollable_overflow_rect();
            auto rect = page().enclosing_device_rect(scrollable_overflow_rect.value());""",
    """            active_document()->update_layout(DOM::UpdateLayoutReason::ProcessScreenshot);
#if defined(BOUCHAUD_PORT)
            // Voir tools/ladybird/prepare-repaint.py. La liste d'affichage est
            // enregistree en coordonnees de fenetre et la surface partagee fait
            // exactement cette taille : le reste du document etait alloue,
            // efface, puis jete a chaque trame.
            auto rect = page().enclosing_device_rect(active_document()->viewport_rect());
            rect.set_location({});
#else
            auto scrollable_overflow_rect = active_document()->layout_node()->paintable_box()->scrollable_overflow_rect();
            auto rect = page().enclosing_device_rect(scrollable_overflow_rect.value());
#endif""",
    "taille de la capture",
)

# ---------------------------------------------------------------------------
# 2. Enfiler une capture sans redemander de trame.
#
# `queue_screenshot_task()` appelle `set_needs_repaint()` et `request_frame()`.
# Appele depuis l'etape de rendu, il rearmerait donc immediatement la trame
# suivante — exactement la boucle qu'on veut supprimer. Il faut une entree qui
# ne fasse qu'enfiler.
# ---------------------------------------------------------------------------

navigable_h = root / "Libraries/LibWeb/HTML/LocalTraversableNavigable.h"
substitute(
    navigable_h,
    """    void process_screenshot_requests();
    void queue_screenshot_task(Optional<UniqueNodeID> node_id)""",
    """    void process_screenshot_requests();
#if defined(BOUCHAUD_PORT)
    // Enfile une capture *sans* `set_needs_repaint()` ni `request_frame()` :
    // appelee depuis l'etape « update the rendering », ou la trame est deja en
    // cours et ou rearmer bouclerait. Voir tools/ladybird/prepare-repaint.py.
    void bouchaud_enqueue_frame_capture() { m_screenshot_tasks.enqueue({ {} }); }
#endif
    void queue_screenshot_task(Optional<UniqueNodeID> node_id)""",
    "enfilage sans rearmement",
)

# ---------------------------------------------------------------------------
# 3. La trame se produit dans l'etape de rendu, sous la garde d'invalidation.
# ---------------------------------------------------------------------------

event_loop = root / "Libraries/LibWeb/HTML/EventLoop/EventLoop.cpp"
substitute(
    event_loop,
    """        navigable->paint_next_frame();
        if (navigable->is_traversable()) {
            auto traversable = navigable->traversable_navigable();
            traversable->process_screenshot_requests();
        }""",
    """        navigable->paint_next_frame();
        if (navigable->is_traversable()) {
            auto traversable = navigable->traversable_navigable();
#if defined(BOUCHAUD_PORT)
            // Bouchaud n'a pas de processus Compositor : `paint_next_frame()`
            // n'a donc personne a qui remettre la liste d'affichage, et la
            // trame se materialise par le chemin capture. On l'enfile ici, dans
            // la boucle deja gardee par `needs_repaint()` juste au-dessus —
            // c'est-a-dire exactement quand le moteur dit qu'il faut repeindre,
            // et jamais autrement.
            //
            // Reserve au navigateur interactif : les scenarios finis M8 et M9
            // choisissent eux-memes leur unique capture et se terminent dessus.
            if (bouchaud_frame_capture_in_rendering_step())
                traversable->bouchaud_enqueue_frame_capture();
#endif
            traversable->process_screenshot_requests();
        }""",
    "capture dans l'etape de rendu",
)

# Le predicat, evalue une seule fois.
substitute(
    event_loop,
    "namespace Web::HTML {\n",
    """namespace Web::HTML {

#if defined(BOUCHAUD_PORT)
static bool bouchaud_frame_capture_in_rendering_step()
{
    static bool const value = getenv("BOUCHAUD_M11") != nullptr;
    return value;
}
#endif
""",
    "predicat de capture",
)

data = event_loop.read_text()
if "#include <cstdlib>" not in data:
    premiere = data.index("#include ")
    event_loop.write_text(data[:premiere] + "#include <cstdlib>\n" + data[premiere:])

print("Repeinture pilotee par l'invalidation appliquee a", root)
