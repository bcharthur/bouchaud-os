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

## 3. On repeignait toute la fenetre pour un curseur qui clignote

Les deux corrections ci-dessus ont rendu au moteur son modele d'invalidation :
sur une page vraiment statique, le compteur de trames ne bouge plus. Restait
le cas d'une page qui change *un peu*. Mesure sur la page d'accueil de Google,
dont le champ de recherche prend le focus tout seul et dont le curseur clignote
donc deux fois par seconde :

    M11_RENDER_STATS full=312 toolbar=40 page=312 pixels=486703296
    PERF-BROWSER pid=5 frames_delta=61 inputs_delta=0 bottleneck=memory-pagefault

312 recompositions completes en trois minutes sans une seule entree. Trois
gachis empiles, tous supprimes ici :

* **le degat etait calcule puis jete.** `record_display_list_and_scroll_state()`
  sait dire quel rectangle a change ; il ne servait qu'au Compositor. La
  capture, elle, recopiait 1 554 048 pixels et annoncait toute la surface.
  Il est desormais accumule et voyage avec la capture (partie 2c).

* **la config de peinture de la capture differait de celle du rendu.** Elles se
  comparent par egalite : une difference d'un pixel faisait reenregistrer TOUTE
  la liste d'affichage a chaque capture, et empechait le calcul de degat de se
  declencher. Elles sont maintenant identiques mot pour mot (partie 1).

* **chaque trame allouait deux fois sa cible.** `Gfx::Bitmap::create()` alloue
  de la memoire ordinaire, `to_shareable_bitmap()` en alloue une seconde,
  anonyme, pour y recopier la premiere. Six mebioctets de pages neuves par
  trame. La cible est desormais unique, anonyme des l'origine, et reutilisee.

Le choix des rectangles cote chrome vit dans
`tools/ladybird/chrome/BouchaudDegat.h` et se verifie sur l'hote, sans QEMU.

## Ce que cela ne change pas

Les scenarios finis M8 et M9 capturent toujours **une** trame, a l'endroit
qu'ils choisissent, et se terminent dessus. L'enfilage automatique de l'etape 1
n'est actif que pour le navigateur interactif (`BOUCHAUD_M11`) : un test
deterministe ne veut pas d'une trame supplementaire decidee par le moteur, ni
d'une cible de capture partagee avec la trame d'avant.
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
# `paint_next_frame()` appartient a LocalNavigable, pas au traversable : c'est
# la que le degat est calcule, donc la qu'il faut le retenir.
local_navigable_cpp = root / "Libraries/LibWeb/HTML/LocalNavigable.cpp"
substitute(
    navigable_cpp,
    """            active_document()->update_layout(DOM::UpdateLayoutReason::ProcessScreenshot);
            auto scrollable_overflow_rect = active_document()->layout_node()->paintable_box()->scrollable_overflow_rect();
            auto rect = page().enclosing_device_rect(scrollable_overflow_rect.value());""",
    """            active_document()->update_layout(DOM::UpdateLayoutReason::ProcessScreenshot);
#if defined(BOUCHAUD_PORT)
            // Voir tools/ladybird/prepare-repaint.py.
            //
            // 1. La liste d'affichage est enregistree en coordonnees de fenetre
            //    et la surface partagee fait exactement cette taille : le reste
            //    du document etait alloue, efface, puis jete a chaque trame.
            //
            // 2. Pour une trame M11, la conversion est celle de
            //    `paint_next_frame()`, mot pour mot. Ce n'est pas une
            //    coquetterie : `PaintConfig` se compare par egalite, et
            //    `record_display_list_and_scroll_state()` ne reenregistre la
            //    liste d'affichage que si la config a change. Une conversion
            //    differente d'un seul pixel reenregistrait donc TOUTE la liste
            //    a chaque capture -- deux enregistrements complets par trame au
            //    lieu d'un -- et interdisait a `paint_next_frame()` de calculer
            //    le moindre degat, puisque son calcul exige que la config
            //    memorisee soit deja la sienne.
            auto rect = task.bouchaud_interactive_frame
                ? page().css_to_device_rect(this->viewport_rect()).to_type<int>()
                : page().enclosing_device_rect(active_document()->viewport_rect()).to_type<int>();
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
    // BrowserHost possede un vrai Compositor et la capture est asynchrone.
    // Ces deux entrees partagent donc un mini-pump a un seul screenshot en vol :
    // - l'etape de rendu peut enfiler sans rearmer ;
    // - le chrome / les callbacks de navigation peuvent programmer une capture.
    // Toute invalidation qui survient pendant le screenshot est coalescee et
    // provoque exactement une capture de rattrapage a la completion.
    void bouchaud_enqueue_interactive_frame_capture_from_rendering();
    void bouchaud_schedule_interactive_frame_capture();

    // Le degat que `paint_next_frame()` vient de calculer, accumule jusqu'a la
    // prochaine capture, puis lu par PageClient quand l'image revient.
    //
    // Ces trois entrees vivent dans la MEME substitution que les deux
    // ci-dessus, et ce n'est pas de la coquetterie : chaque `substitute()` se
    // garde en cherchant son propre resultat. Une seconde substitution qui
    // viendrait inserer des lignes dans ce bloc effacerait la garde de la
    // premiere, et le script cesserait d'etre rejouable sur un arbre deja
    // prepare -- ce que fait tout rebuild local.
    void bouchaud_accumulate_frame_damage(Gfx::IntRect const&);
    void bouchaud_require_full_frame_damage();
    Gfx::IntRect bouchaud_last_frame_damage() const { return m_bouchaud_last_frame_damage; }
#endif
    void queue_screenshot_task(Optional<UniqueNodeID> node_id)""",
    "enfilage sans rearmement",
)

# ---------------------------------------------------------------------------
# 2b. BrowserHost + M11 : un seul screenshot Compositor en vol.
#
# `paint_next_frame()` met a jour le vrai processus Compositor. La capture
# utilise ensuite `request_screenshot()`, dont le callback est asynchrone. Sur
# une page dynamique, plusieurs invalidations pouvaient donc empiler des
# captures d'etats anciens et la surface Bouchaud finissait par rester sur la
# page precedente alors que navigation/reseau/historique avaient avance.
#
# Le pump ci-dessous distingue :
#   queued   : une tache attend la prochaine etape de rendu ;
#   in_flight: le Compositor calcule actuellement le screenshot ;
#   dirty    : une nouvelle invalidation est arrivee pendant in_flight.
#
# Il n'y a jamais plus d'une capture asynchrone en vol. Quand elle revient,
# `dirty` rearme UNE occasion de rendu, sans timer et sans boucle 60 Hz.
# ---------------------------------------------------------------------------

substitute(
    navigable_h,
    """    struct ScreenshotTask {
        Optional<Web::UniqueNodeID> node_id;
    };
    Queue<ScreenshotTask> m_screenshot_tasks;""",
    """    struct ScreenshotTask {
        Optional<Web::UniqueNodeID> node_id;
#if defined(BOUCHAUD_PORT)
        bool bouchaud_interactive_frame { false };
#endif
    };
    Queue<ScreenshotTask> m_screenshot_tasks;
#if defined(BOUCHAUD_PORT)
    bool m_bouchaud_frame_capture_queued { false };
    bool m_bouchaud_frame_capture_in_flight { false };
    bool m_bouchaud_frame_capture_dirty { false };

    // Degat accumule depuis la derniere capture, en coordonnees de fenetre.
    // `_full` l'emporte : il dit « on ne sait plus », pas « tout a change ».
    Gfx::IntRect m_bouchaud_frame_damage;
    bool m_bouchaud_frame_damage_full { true };

    // Le degat remis avec la capture en vol. Lu par PageClient a la
    // completion, ou la capture et son degat doivent se retrouver.
    Gfx::IntRect m_bouchaud_last_frame_damage;

    // La cible des captures M11, reutilisee d'une trame a l'autre.
    RefPtr<Gfx::Bitmap> m_bouchaud_frame_bitmap;

    ErrorOr<NonnullRefPtr<Gfx::Bitmap>> bouchaud_interactive_frame_bitmap(Gfx::IntSize);
    Gfx::IntRect bouchaud_take_frame_damage(Gfx::IntSize);
    void bouchaud_interactive_frame_capture_started();
    void bouchaud_interactive_frame_capture_completed();
#endif""",
    "etat single-flight M11",
)

# `RefPtr<Gfx::Bitmap>` est un MEMBRE : son destructeur exige le type complet.
data = navigable_h.read_text()
if "#include <LibGfx/Bitmap.h>" not in data:
    ancre = "#include <LibWeb/HTML/LocalNavigable.h>"
    if ancre not in data:
        raise SystemExit("repaint : ancre d'inclusion LibGfx introuvable")
    navigable_h.write_text(data.replace(ancre, "#include <LibGfx/Bitmap.h>\n" + ancre, 1))

substitute(
    navigable_cpp,
    """void LocalTraversableNavigable::process_screenshot_requests()
{""",
    """#if defined(BOUCHAUD_PORT)
void LocalTraversableNavigable::bouchaud_enqueue_interactive_frame_capture_from_rendering()
{
    // Rien n'a change VISUELLEMENT : `compute_display_list_damage()` vient de
    // rendre un rectangle vide. Une capture couterait alors un readback complet
    // du Compositor, une image de trois mebioctets et une recopie de surface --
    // pour reafficher exactement les memes pixels.
    //
    // C'est le cas courant sur une page qui execute du script sans rien
    // repeindre. L'appel explicite (`bouchaud_schedule_...`) n'a pas cette
    // garde : une navigation ou un changement de chrome demande sa trame, il ne
    // la deduit pas.
    if (!m_bouchaud_frame_damage_full && m_bouchaud_frame_damage.is_empty())
        return;

    // Si une tache est deja en file, cette etape de rendu est justement celle
    // qui va la servir : ne pas creer un doublon et ne pas marquer dirty.
    if (m_bouchaud_frame_capture_queued)
        return;

    // Le Compositor travaille deja sur une image. La display-list vient d'etre
    // rafraichie par paint_next_frame(); retenir seulement qu'il faudra relire
    // le dernier etat lorsque le callback courant reviendra.
    if (m_bouchaud_frame_capture_in_flight) {
        m_bouchaud_frame_capture_dirty = true;
        return;
    }

    m_bouchaud_frame_capture_queued = true;
    m_screenshot_tasks.enqueue({ {}, true });
}

void LocalTraversableNavigable::bouchaud_schedule_interactive_frame_capture()
{
    // Les callbacks M11 (navigation commencee/commitee/terminee, chrome) peuvent
    // se telescoper. Une capture deja en file sera faite APRES le prochain
    // paint, donc elle verra deja l'etat le plus recent.
    if (m_bouchaud_frame_capture_queued)
        return;

    if (m_bouchaud_frame_capture_in_flight) {
        m_bouchaud_frame_capture_dirty = true;
        return;
    }

    m_bouchaud_frame_capture_queued = true;
    m_screenshot_tasks.enqueue({ {}, true });
    set_needs_repaint();
    page().client().request_frame();
}

void LocalTraversableNavigable::bouchaud_interactive_frame_capture_started()
{
    m_bouchaud_frame_capture_queued = false;
    m_bouchaud_frame_capture_in_flight = true;
}

void LocalTraversableNavigable::bouchaud_interactive_frame_capture_completed()
{
    if (!m_bouchaud_frame_capture_in_flight)
        return;

    m_bouchaud_frame_capture_in_flight = false;
    if (!m_bouchaud_frame_capture_dirty)
        return;

    // Des changements sont arrives pendant le screenshot. En demander
    // exactement un autre via la boucle de rendu normale. Pas de timer, pas de
    // capture recursive depuis le callback IPC.
    m_bouchaud_frame_capture_dirty = false;
    bouchaud_schedule_interactive_frame_capture();
}

void LocalTraversableNavigable::bouchaud_accumulate_frame_damage(Gfx::IntRect const& damage)
{
    // « Tout » l'emporte : c'est un aveu d'ignorance, pas un rectangle. Le
    // reunir avec un rectangle plus petit le retrecirait.
    if (m_bouchaud_frame_damage_full)
        return;
    if (damage.is_empty())
        return;
    // `united()` d'un rectangle vide engloberait l'origine, et une trame dont
    // seul le coin bas-droit a change repeindrait toute la fenetre.
    m_bouchaud_frame_damage = m_bouchaud_frame_damage.is_empty()
        ? damage
        : m_bouchaud_frame_damage.united(damage);
}

void LocalTraversableNavigable::bouchaud_require_full_frame_damage()
{
    m_bouchaud_frame_damage_full = true;

    // Un navigable imbrique peut peindre APRES que la capture du sommet est
    // partie : cette capture porte alors un etat perime, et rien dans l'etape
    // de rendu ne le rattrape -- une iframe n'est pas traversable, elle
    // n'enfile donc pas de capture. Marquer sale garantit exactement une
    // capture de rattrapage a la completion, sans timer et sans boucle.
    if (m_bouchaud_frame_capture_in_flight)
        m_bouchaud_frame_capture_dirty = true;
}

Gfx::IntRect LocalTraversableNavigable::bouchaud_take_frame_damage(Gfx::IntSize taille)
{
    Gfx::IntRect complet { {}, taille };
    auto degat = m_bouchaud_frame_damage_full
        ? complet
        : m_bouchaud_frame_damage.intersected(complet);
    m_bouchaud_frame_damage_full = false;
    m_bouchaud_frame_damage = {};
    return degat;
}

ErrorOr<NonnullRefPtr<Gfx::Bitmap>> LocalTraversableNavigable::bouchaud_interactive_frame_bitmap(Gfx::IntSize taille)
{
    if (m_bouchaud_frame_bitmap && m_bouchaud_frame_bitmap->size() == taille)
        return NonnullRefPtr<Gfx::Bitmap> { *m_bouchaud_frame_bitmap };

    // `create_shareable()` alloue DEJA dans un tampon anonyme, si bien que
    // `to_shareable_bitmap()` rendra le meme objet au lieu d'en allouer un
    // second et d'y recopier trois mebioctets.
    m_bouchaud_frame_bitmap = TRY(Gfx::Bitmap::create_shareable(
        Gfx::BitmapFormat::BGRA8888, Gfx::AlphaType::Premultiplied, taille));

    // La cible a change de taille : la surface partagee ne porte plus la trame
    // precedente, et un degat partiel n'aurait plus de sens.
    bouchaud_require_full_frame_damage();
    return NonnullRefPtr<Gfx::Bitmap> { *m_bouchaud_frame_bitmap };
}
#endif

void LocalTraversableNavigable::process_screenshot_requests()
{""",
    "implementation single-flight M11",
)

substitute(
    navigable_cpp,
    """            auto bitmap_or_error = Gfx::Bitmap::create(Gfx::BitmapFormat::BGRA8888, rect.size().to_type<int>());
            if (bitmap_or_error.is_error()) {
                client.page_did_take_screenshot({});
                continue;
            }
            auto bitmap = bitmap_or_error.release_value();
            auto painting_surface = Gfx::PaintingSurface::wrap_bitmap(*bitmap);
            PaintConfig paint_config { .paint_overlay = true, .canvas_fill_rect = rect.to_type<int>() };
            render_screenshot(painting_surface, paint_config, [bitmap, &client] {
                client.page_did_take_screenshot(bitmap->to_shareable_bitmap());
            });""",
    """#if defined(BOUCHAUD_PORT)
            auto const bouchaud_interactive_frame = task.bouchaud_interactive_frame;
            // Voir tools/ladybird/prepare-repaint.py : la cible d'une capture
            // M11 est REUTILISEE d'une trame a l'autre.
            //
            // `Gfx::Bitmap::create()` alloue de la memoire ordinaire, puis
            // `to_shareable_bitmap()` en alloue une SECONDE, anonyme, pour y
            // recopier la premiere. Trois mebioctets de pages neuves deux fois
            // par trame : c'est ce chemin que le journal de la machine
            // designait par « bottleneck=memory-pagefault ».
            //
            // Un seul tampon, pas deux en alternance : si le Compositor ne
            // repeignait que le degat au lieu de toute la surface, un tampon
            // unique resterait coherent -- il porte deja la trame precedente --
            // la ou deux tampons alternes porteraient l'avant-derniere.
            auto bitmap_or_error = bouchaud_interactive_frame
                ? bouchaud_interactive_frame_bitmap(rect.size().to_type<int>())
                : Gfx::Bitmap::create(Gfx::BitmapFormat::BGRA8888, rect.size().to_type<int>());
            if (bouchaud_interactive_frame) {
                bouchaud_interactive_frame_capture_started();
                // La capture et son degat partent ensemble : PageClient lira ce
                // rectangle quand le Compositor rendra l'image.
                m_bouchaud_last_frame_damage = bouchaud_take_frame_damage(rect.size().to_type<int>());
            } else {
                // Une capture qui n'est pas une trame M11 -- outil de
                // developpement, scenario fini -- ne doit pas heriter du degat
                // de la trame precedente : elle vaut pour toute sa surface.
                m_bouchaud_last_frame_damage = Gfx::IntRect { {}, rect.size().to_type<int>() };
            }
#else
            auto bitmap_or_error = Gfx::Bitmap::create(Gfx::BitmapFormat::BGRA8888, rect.size().to_type<int>());
#endif
            if (bitmap_or_error.is_error()) {
                client.page_did_take_screenshot({});
#if defined(BOUCHAUD_PORT)
                if (bouchaud_interactive_frame)
                    bouchaud_interactive_frame_capture_completed();
#endif
                continue;
            }
            auto bitmap = bitmap_or_error.release_value();
            auto painting_surface = Gfx::PaintingSurface::wrap_bitmap(*bitmap);
            PaintConfig paint_config { .paint_overlay = true, .canvas_fill_rect = rect.to_type<int>() };
            render_screenshot(painting_surface, paint_config, [bitmap, &client
#if defined(BOUCHAUD_PORT)
                , this, bouchaud_interactive_frame
#endif
            ] {
                client.page_did_take_screenshot(bitmap->to_shareable_bitmap());
#if defined(BOUCHAUD_PORT)
                if (bouchaud_interactive_frame)
                    bouchaud_interactive_frame_capture_completed();
#endif
            });""",
    "completion screenshot M11",
)

# ---------------------------------------------------------------------------
# 2c. Le degat calcule par `paint_next_frame()` cesse d'etre jete.
#
# `record_display_list_and_scroll_state()` compare la nouvelle liste
# d'affichage a celle de la trame precedente et en tire le rectangle qui a
# reellement change. Ce rectangle partait au Compositor et nulle part ailleurs :
# la capture, elle, recopiait la fenetre entiere et l'annoncait entierement.
#
# Il est ACCUMULE, pas ecrase. Le pump ne garde qu'une capture en vol : les
# etapes de rendu qui tombent pendant une capture ont bel et bien change des
# pixels, et ne retenir que la derniere laisserait leurs traces a l'ecran.
# ---------------------------------------------------------------------------

substitute(
    local_navigable_cpp,
    """    viewport_rect = page().css_to_device_rect(this->viewport_rect()).to_type<int>();
    compositor_context().present_frame(viewport_rect, damage_rect);
}""",
    """    viewport_rect = page().css_to_device_rect(this->viewport_rect()).to_type<int>();
    compositor_context().present_frame(viewport_rect, damage_rect);

#if defined(BOUCHAUD_PORT)
    // Voir tools/ladybird/prepare-repaint.py.
    if (page().top_level_traversable_is_initialized()) {
        auto traversable = page().top_level_traversable();
        if (traversable.ptr() == this) {
            traversable->bouchaud_accumulate_frame_damage(damage_rect);
        } else {
            // Un navigable imbrique peint dans le contexte de son parent : son
            // rectangle est dans SON repere. Le reunir avec celui du sommet
            // designerait des pixels au hasard. Une iframe qui bouge coute donc
            // une trame complete -- exact, et bien plus rare qu'un curseur qui
            // clignote.
            traversable->bouchaud_require_full_frame_damage();
        }
    }
#endif
}""",
    "accumulation du degat M11",
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
            // BrowserHost possede maintenant un vrai processus Compositor.
            // `paint_next_frame()` lui remet d'abord la display-list courante ;
            // la capture M11 est ensuite un readback asynchrone de ce meme etat.
            // Le helper single-flight ci-dessous coalesce les invalidations
            // pendant le readback au lieu d'empiler des screenshots anciens.
            //
            // Reserve au navigateur interactif : les scenarios finis M8 et M9
            // choisissent eux-memes leur unique capture et se terminent dessus.
            if (bouchaud_frame_capture_in_rendering_step())
                traversable->bouchaud_enqueue_interactive_frame_capture_from_rendering();
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
