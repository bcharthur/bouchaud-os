#!/usr/bin/env python3
"""M11 — le chrome du navigateur, greffe dans WebContent.

S'execute APRES `prepare-m9-source.py` : toutes les ancres utilisees ici sont
soit du texte upstream stable, soit du texte que **nos propres** scripts ont
ecrit juste avant. C'est deliberé — une ancre que nous produisons ne peut pas
diverger d'upstream sans que nous le sachions.

Ce que M11 ajoute a M9, et rien de plus :

  * une barre d'outils (reculer / avancer / recharger / barre d'adresse) peinte
    au-dessus de la page, dans la meme surface partagee ;
  * les evenements du gestionnaire de fenetres (`Key`, `Pointer`, `Wheel`)
    convertis en `Web::MouseEvent` / `Web::KeyEvent` et remis a la file d'entree
    normale de WebContent ;
  * un repeint a chaque changement, au lieu d'une capture unique.

Le code du chrome lui-meme vit dans `tools/ladybird/chrome/BouchaudChrome.h`,
copie tel quel dans l'arbre jetable : c'est un fichier relisible en revue, pas
une chaine de caracteres dans un script.
"""

from pathlib import Path
import shutil
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-m11-chrome.py <ladybird-worktree>")

root = Path(sys.argv[1])
here = Path(__file__).resolve().parent


def substitute(path: Path, old: str, new: str, label: str) -> str:
    data = path.read_text()
    if new in data:
        return data
    if old not in data:
        raise SystemExit(f"M11 : ancre introuvable ({label}) dans {path}")
    data = data.replace(old, new, 1)
    path.write_text(data)
    return data


def ensure_include(path: Path, include: str, anchor: str, label: str) -> None:
    data = path.read_text()
    if include in data:
        return
    if anchor not in data:
        raise SystemExit(f"M11 : ancre d'inclusion introuvable ({label}) dans {path}")
    path.write_text(data.replace(anchor, include + "\n" + anchor, 1))


# ---------------------------------------------------------------------------
# 1. Le chrome, copie tel quel.
# ---------------------------------------------------------------------------

source_header = here / "chrome" / "BouchaudChrome.h"
if not source_header.is_file():
    raise SystemExit(f"M11 : {source_header} absent")

destination = root / "Services/WebContent/BouchaudChrome.h"
shutil.copyfile(source_header, destination)

# En-tete seul : aucune modification de `Services/WebContent/CMakeLists.txt`,
# donc aucune divergence de construction de plus avec l'arbre epingle.


# ---------------------------------------------------------------------------
# 2. ConnectionFromClient : declaration, membres, demarrage.
# ---------------------------------------------------------------------------

connection_h = root / "Services/WebContent/ConnectionFromClient.h"
substitute(
    connection_h,
    """#if defined(BOUCHAUD_PORT)
    void bouchaud_m8_start();
    void bouchaud_m9_start();
#endif""",
    """#if defined(BOUCHAUD_PORT)
    void bouchaud_m8_start();
    void bouchaud_m9_start();
    void bouchaud_m11_start();

    // Le canal GUI est lu par un notificateur **et** par un minuteur. La sonde
    // M9 a montre qu'un notificateur de lecture pouvait rester endormi sous
    // Bouchaud alors que la donnee etait deja arrivee ; un navigateur dont la
    // souris s'arrete par intermittence n'est pas utilisable, et le minuteur est
    // la ceinture qui garantit que l'entree finit toujours par etre servie.
    RefPtr<Core::Notifier> m_bouchaud_gui_notifier;
    RefPtr<Core::Timer> m_bouchaud_gui_timer;
#endif""",
    "declaration M11",
)

ensure_include(
    connection_h,
    "#include <LibCore/Notifier.h>\n#include <LibCore/Timer.h>",
    "#include <LibGC/Root.h>",
    "includes LibCore ConnectionFromClient.h",
)


connection_cpp = root / "Services/WebContent/ConnectionFromClient.cpp"

ensure_include(
    connection_cpp,
    "#if defined(BOUCHAUD_PORT)\n#    include <WebContent/BouchaudChrome.h>\n#endif",
    "#include <WebContent/PageClient.h>",
    "include BouchaudChrome ConnectionFromClient.cpp",
)

# Le viewport de la page s'arrete au-dessus de la barre d'outils. La conversion
# vit dans le chrome : si la hauteur de la barre change un jour, elle change a
# un seul endroit, et le clic continue de tomber ou le pixel a ete peint.
substitute(
    connection_cpp,
    """    outln("[ladybird-bouchaud] M9_STAGE initialize ok");

    auto viewport = Gfx::IntSize { width, height }.to_type<Web::DevicePixels>();
    auto screen = Gfx::IntRect { 0, 0, width, height }.to_type<Web::DevicePixels>();""",
    """    outln("[ladybird-bouchaud] M9_STAGE initialize ok");

    auto page_height = height;
    if (BouchaudChrome::enabled()) {
        BouchaudChrome::initialize_from_environment();
        page_height = BouchaudChrome::viewport_height();
    }

    auto viewport = Gfx::IntSize { width, page_height }.to_type<Web::DevicePixels>();
    auto screen = Gfx::IntRect { 0, 0, width, height }.to_type<Web::DevicePixels>();""",
    "viewport M11",
)

# Le demarrage du chrome precede la navigation : la barre doit deja porter
# l'URL demandee quand la premiere reponse arrive, sinon le premier repeint
# affiche une barre vide sous une page chargee.
substitute(
    connection_cpp,
    """    outln("[ladybird-bouchaud] M9_BOOTSTRAP page={} viewport={}x{}", page_id, width, height);
    outln("[ladybird-bouchaud] M9_NAVIGATION_BEGIN url={}", *url);""",
    """    outln("[ladybird-bouchaud] M9_BOOTSTRAP page={} viewport={}x{}", page_id, width, page_height);

    if (BouchaudChrome::enabled()) {
        BouchaudChrome::set_committed_url(url->to_byte_string());
        bouchaud_m11_start();
    }

    outln("[ladybird-bouchaud] M9_NAVIGATION_BEGIN url={}", *url);""",
    "demarrage M11",
)

m11_start = r'''
void ConnectionFromClient::bouchaud_m11_start()
{
    constexpr u64 page_id = 1;
    auto& chrome = BouchaudChrome::state();

    // Les rappels traversent l'objet global du chrome et capturent `this`. Sous
    // M9/M11, WebContent n'a qu'une `ConnectionFromClient` et elle vit aussi
    // longtemps que le processus : la capture est sure tant que ce jalon ne cree
    // pas de second onglet, ce que M13 fera en introduisant un vrai Browser.
    chrome.on_mouse_event = [this, page_id](Web::MouseEvent event) {
        mouse_event(page_id, move(event));
    };

    chrome.on_key_event = [this, page_id](Web::KeyEvent event) {
        key_event(page_id, move(event));
    };

    chrome.on_navigate = [this, page_id](ByteString target) {
        auto url = URL::create_with_url_or_path(target);
        if (!url.has_value()) {
            warnln("[ladybird-bouchaud] M11_URL_INVALID {}", target);
            BouchaudChrome::set_loading(false, "URL invalide"sv);
            return;
        }
        load_url(page_id, *url, Web::Bindings::NavigationHistoryBehavior::Auto);
    };

    chrome.on_history_delta = [this, page_id](int delta) {
        if (auto page = this->page(page_id); page.has_value())
            page->page().traverse_the_history_by_delta(delta);
    };

    chrome.on_reload = [this, page_id] {
        reload(page_id);
    };

    chrome.on_stop = [this, page_id] {
        stop_loading(page_id);
    };

    chrome.on_repaint = [this, page_id] {
        if (auto page = this->page(page_id); page.has_value())
            page->page().top_level_traversable()->queue_screenshot_task({});
    };

    chrome.on_close = [] {
        outln("[ladybird-bouchaud] M11_EXIT demande du gestionnaire de fenetres");
        Core::Process::terminate_immediately(0);
    };

    if (chrome.gui_fd >= 0) {
        m_bouchaud_gui_notifier = Core::Notifier::construct(chrome.gui_fd, Core::Notifier::Type::Read);
        m_bouchaud_gui_notifier->on_activation = [] {
            BouchaudChrome::drain();
        };
    }

    // Bouchaud n'ordonnance que sur le processeur d'amorcage : chaque trame
    // est prise au moteur, pas ajoutee a cote. Trente par seconde suffisent a
    // un defilement fluide et laissent la moitie du cœur au reste — analyse,
    // script, reseau. C'est LibWeb qui fait respecter ce plafond, dans
    // `PageClient::request_frame()`.
    if (auto page = this->page(page_id); page.has_value())
        page->set_maximum_frames_per_second(30.0);

    // 16 ms : la cadence du bureau (`docs/GUI_USERLAND_PROTOCOL.md` §7). Ce
    // minuteur ne demande plus de trame de page — il lit les entrees et
    // recompose la barre d'outils quand elle a change. Voir
    // tools/ladybird/prepare-repaint.py.
    m_bouchaud_gui_timer = Core::Timer::create_repeating(16, [] {
        BouchaudChrome::tick();
    });
    m_bouchaud_gui_timer->start();

    outln("[ladybird-bouchaud] M11_READY toolbar={} viewport_height={}",
        BouchaudChrome::toolbar_height, BouchaudChrome::viewport_height());
}
#endif
'''

substitute(
    connection_cpp,
    """    load_url(page_id, *url, Web::Bindings::NavigationHistoryBehavior::Auto);
}
#endif
""",
    """    load_url(page_id, *url, Web::Bindings::NavigationHistoryBehavior::Auto);
}
""" + m11_start,
    "corps M11",
)


# ---------------------------------------------------------------------------
# 3. PageClient : composition, etat de chargement, titre.
# ---------------------------------------------------------------------------

page_cpp = root / "Services/WebContent/PageClient.cpp"

ensure_include(
    page_cpp,
    "#if defined(BOUCHAUD_PORT)\n#    include <WebContent/BouchaudChrome.h>\n#endif",
    "#include <WebContent/WebUIConnection.h>",
    "include BouchaudChrome PageClient.cpp",
)

# La capture part vers le chrome, qui compose barre d'outils + page. M9 sans
# M11 conserve exactement son chemin : la comparaison de capture de la CI ne
# change pas de reference.
substitute(
    page_cpp,
    """    if (bouchaud_m9_enabled()) {
        if (!bouchaud_m9_present(screenshot))
            Core::Process::terminate_immediately(70);

        outln("[ladybird-bouchaud] M9_CPU_SCREENSHOT_RENDERED");""",
    """    if (bouchaud_m9_enabled()) {
        if (BouchaudChrome::enabled()) {
            if (!BouchaudChrome::present(screenshot))
                Core::Process::terminate_immediately(70);
            return;
        }

        if (!bouchaud_m9_present(screenshot))
            Core::Process::terminate_immediately(70);

        outln("[ladybird-bouchaud] M9_CPU_SCREENSHOT_RENDERED");""",
    "composition M11",
)

# Fin de chargement. M9 ne capture que le document attendu, parce qu'il n'a
# qu'une URL et qu'un `about:blank` intermediaire ferait echouer son invariant.
# M11 navigue : *chaque* document commite est celui qu'il faut montrer.
substitute(
    page_cpp,
    """    if (bouchaud_m9_enabled()) {
        // `page_did_finish_loading` se declenche pour **chaque** document, y""",
    """    if (bouchaud_m9_enabled() && BouchaudChrome::enabled()) {
        auto chargee = url.to_byte_string();
        // Le `about:blank` initial se termine pendant que la vraie navigation
        // est encore en vol. L'afficher dans la barre d'adresse ferait clignoter
        // une URL que l'utilisateur n'a pas demandee, juste avant la sienne.
        if (chargee == "about:blank") {
            outln("[ladybird-bouchaud] M11_DOCUMENT_SKIPPED url={}", chargee);
            return;
        }
        BouchaudChrome::set_committed_url(chargee);
        BouchaudChrome::set_loading(false, "pret"sv);
        outln("[ladybird-bouchaud] M11_DOCUMENT_LOADED page={} url={}", m_id, chargee);
        page().top_level_traversable()->queue_screenshot_task({});
        return;
    }
    if (bouchaud_m9_enabled()) {
        // `page_did_finish_loading` se declenche pour **chaque** document, y""",
    "fin de chargement M11",
)

substitute(
    page_cpp,
    """    if (bouchaud_m9_enabled()) {
        outln("[ladybird-bouchaud] M9_NAVIGATION_STARTED page={} url={} redirect={}", m_id, url, is_redirect);
        return;
    }""",
    """    if (bouchaud_m9_enabled()) {
        outln("[ladybird-bouchaud] M9_NAVIGATION_STARTED page={} url={} redirect={}", m_id, url, is_redirect);
        if (BouchaudChrome::enabled()) {
            BouchaudChrome::set_committed_url(url.to_byte_string());
            BouchaudChrome::set_loading(true, "chargement..."sv);
            page().top_level_traversable()->queue_screenshot_task({});
        }
        return;
    }""",
    "debut de chargement M11",
)

substitute(
    page_cpp,
    """    if (bouchaud_m9_enabled()) {
        outln("[ladybird-bouchaud] M9_NAVIGATION_COMMITTED page={} url={}", m_id, url);
        return;
    }""",
    """    if (bouchaud_m9_enabled()) {
        outln("[ladybird-bouchaud] M9_NAVIGATION_COMMITTED page={} url={}", m_id, url);
        if (BouchaudChrome::enabled()) {
            BouchaudChrome::set_committed_url(url.to_byte_string());
            page().top_level_traversable()->queue_screenshot_task({});
        }
        return;
    }""",
    "URL commitee M11",
)

# Un chargement annule doit se voir : sans cela, la barre reste sur
# « chargement... » pour toujours et l'utilisateur attend une page qui ne
# viendra pas.
substitute(
    page_cpp,
    """    if (bouchaud_m9_enabled())
        outln("[ladybird-bouchaud] M9_NAVIGATION_CANCELLED page={} url={}", m_id, url);""",
    """    if (bouchaud_m9_enabled()) {
        outln("[ladybird-bouchaud] M9_NAVIGATION_CANCELLED page={} url={}", m_id, url);
        if (BouchaudChrome::enabled()) {
            BouchaudChrome::set_loading(false, "chargement interrompu"sv);
            page().top_level_traversable()->queue_screenshot_task({});
        }
    }""",
    "annulation M11",
)

substitute(
    page_cpp,
    """void PageClient::page_did_change_title(Utf16String const& title)
{
    client().async_did_change_title(m_id, title);
}""",
    """void PageClient::page_did_change_title(Utf16String const& title)
{
#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled()) {
        if (BouchaudChrome::enabled())
            BouchaudChrome::set_title(ByteString::formatted("{}", title));
        return;
    }
#endif
    client().async_did_change_title(m_id, title);
}""",
    "titre M11",
)

print("Bouchaud M11 chrome applique a", root)
