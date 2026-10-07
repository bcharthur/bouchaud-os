/*
 * Bouchaud OS -- UI/Bouchaud : la fenetre du navigateur.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/StringBuilder.h>
#include <AK/Time.h>
#include <stdlib.h>
#include <LibCore/EventLoop.h>
#include <LibCore/Notifier.h>
#include <LibCore/Timer.h>
#include <LibGfx/Bitmap.h>
#include <LibURL/Parser.h>
#include <LibWebView/ConsoleOutput.h>
#include <LibWebView/Menu.h>
#include <UI/Bouchaud/BouchaudChrome.h>
#include <UI/Bouchaud/BrowserWindow.h>

namespace BouchaudUI {

static constexpr u64 trames_journalisees = 4096;
static constexpr u64 releve_toutes_les = 256;

BrowserWindow::BrowserWindow(Core::AnonymousBuffer theme, Web::DevicePixelSize viewport)
    : m_theme(move(theme))
    , m_viewport(viewport)
{
}

BrowserWindow::~BrowserWindow()
{
    // Les rappels du chrome capturent `this` : un chrome qui survivrait a la
    // fenetre appellerait dans le vide.
    auto& c = BouchaudChrome::state();
    c.on_mouse_event = nullptr;
    c.on_key_event = nullptr;
    c.on_navigate = nullptr;
    c.on_history_delta = nullptr;
    c.on_reload = nullptr;
    c.on_stop = nullptr;
    c.on_repaint = nullptr;
    c.on_resize = nullptr;
    c.on_zoom = nullptr;
    c.on_nouvel_onglet = nullptr;
    c.on_fermer_onglet = nullptr;
    c.on_find = nullptr;
    c.on_find_next = nullptr;
    c.on_find_previous = nullptr;
    c.on_select_all = nullptr;
    c.on_copy = nullptr;
    c.on_cut = nullptr;
    c.on_paste = nullptr;
    c.on_close = nullptr;
    c.on_onglet_actif = nullptr;
}

BouchaudWebView* BrowserWindow::vue_de_l_onglet(u64 onglet) const
{
    for (auto& vue : m_vues) {
        if (vue->onglet() == onglet)
            return vue.ptr();
    }
    return nullptr;
}

BouchaudWebView* BrowserWindow::vue_active() const
{
    return vue_de_l_onglet(BouchaudChrome::page_active());
}

void BrowserWindow::demarre(URL::URL const& url_initiale)
{
    if (BouchaudChrome::enabled()) {
        BouchaudChrome::initialize_from_environment();
        // La page commence sous la barre d'outils et la bande d'onglets.
        m_viewport = Web::DevicePixelSize { BouchaudChrome::state().surface_width, BouchaudChrome::viewport_height() };
    }
    branche_chrome();

    auto const premier = ouvre_onglet(url_initiale, true, true);
    warnln("[LB:UI] premier_onglet={} viewport={}x{} url={}", premier,
        m_viewport.width().value(), m_viewport.height().value(), url_initiale);

    if (!BouchaudChrome::enabled()) {
        warnln("[LB:UI] sans_fenetre=1 raison=BO_GUI_FD/BO_SURFACE_FD absents");
        return;
    }

    auto const fd = BouchaudChrome::state().gui_fd;
    if (fd >= 0) {
        m_notificateur_gui = Core::Notifier::construct(fd, Core::Notifier::Type::Read);
        m_notificateur_gui->on_activation = [] {
            BouchaudChrome::drain();
        };
    }

    // 16 ms : la cadence du bureau (`docs/GUI_USERLAND_PROTOCOL.md` §7). Ce
    // minuteur lit les entrees en retard et recompose le CHROME quand il a
    // change ; il ne demande jamais de trame de page.
    m_tic = Core::Timer::create_repeating(16, [] {
        BouchaudChrome::tick();
    });
    m_tic->start();
    warnln("[LB:UI] canal_gui=notifier tic_ms=16");
}

u64 BrowserWindow::ouvre_onglet(URL::URL const& url, bool activer, bool annonce_au_chrome)
{
    auto const onglet = m_prochain_onglet++;
    auto vue = BouchaudWebView::create(*this, onglet, m_theme, m_viewport);
    branche_vue(*vue);
    vue->set_system_visibility_state(activer ? Web::HTML::VisibilityState::Visible : Web::HTML::VisibilityState::Hidden);
    vue->load(url);
    warnln("[LB:TAB] ouvert onglet={} page={} processus=neuf", onglet, vue->page_courante());
    m_vues.append(move(vue));
    if (annonce_au_chrome)
        BouchaudChrome::ajoute_onglet(onglet, url.serialize().to_byte_string(), activer);
    return onglet;
}

String BrowserWindow::ouvre_vue_demandee_par_la_page(BouchaudWebView& ouvreur, Web::HTML::ActivateTab activer, Optional<u64> page_index)
{
    auto const onglet = m_prochain_onglet++;
    auto vue = page_index.has_value()
        ? BouchaudWebView::create_child(*this, onglet, ouvreur, *page_index)
        : BouchaudWebView::create(*this, onglet, m_theme, m_viewport);
    branche_vue(*vue);
    auto handle = vue->handle();
    warnln("[LB:TAB] ouvert onglet={} page={} ouvreur={} processus={}", onglet, vue->page_courante(),
        ouvreur.onglet(), page_index.has_value() ? "ouvreur"sv : "neuf"sv);
    m_vues.append(move(vue));
    BouchaudChrome::ajoute_onglet(onglet, ByteString {}, activer == Web::HTML::ActivateTab::Yes);
    return handle;
}

void BrowserWindow::retire_vue(u64 onglet)
{
    for (size_t i = 0; i < m_vues.size(); ++i) {
        if (m_vues[i]->onglet() != onglet)
            continue;
        warnln("[LB:TAB] ferme onglet={} page={}", onglet, m_vues[i]->page_courante());
        m_vues.remove(i);
        break;
    }
    // Le chrome rappelle `on_close` s'il n'y a plus d'onglet.
    BouchaudChrome::retire_onglet(onglet);
}

// BOUCHAUD_SONDE_PIXELS_V1 -- ce que le Compositor a REELLEMENT peint.
//
// « page blanche » ne se deduit pas du DOM : un document peut etre charge et
// rien peint, ou peint uniformement. La sonde lit 64 x 48 points de la trame
// presentee (le bitmap du Compositor, avant toute copie) : somme de controle,
// luminance moyenne et variance, nombre de couleurs distinctes (borne a 64),
// part de points non blancs. Une page reelle a de la variance et plusieurs
// couleurs ; une trame vide ou uniforme n'en a pas.
struct SondePixels {
    u32 somme { 2166136261u };
    u32 luminance_moyenne { 0 };
    u32 variance { 0 };
    u32 couleurs { 0 };
    u32 non_blanc_pct { 0 };
    u32 echantillons { 0 };
};

static SondePixels sonde_pixels(Gfx::Bitmap const& bitmap, int largeur, int hauteur)
{
    SondePixels s;
    int const w = min(largeur, bitmap.width());
    int const h = min(hauteur, bitmap.height());
    if (w <= 0 || h <= 0)
        return s;
    u32 vues[64];
    u32 nb_vues = 0;
    u64 total = 0;
    u64 carres = 0;
    u32 non_blanc = 0;
    for (int j = 0; j < 48; ++j) {
        int const y = (j * 2 + 1) * h / 96;
        auto const* ligne = bitmap.scanline(y);
        for (int i = 0; i < 64; ++i) {
            int const x = (i * 2 + 1) * w / 128;
            u32 const p = ligne[x] & 0x00ffffff;
            u32 const r = (p >> 16) & 0xff, g = (p >> 8) & 0xff, b = p & 0xff;
            u32 const l = (r * 299 + g * 587 + b * 114) / 1000;
            total += l;
            carres += static_cast<u64>(l) * l;
            non_blanc += l < 245;
            s.somme = (s.somme ^ p) * 16777619u;
            bool deja = false;
            for (u32 k = 0; k < nb_vues && !deja; ++k)
                deja = vues[k] == p;
            if (!deja && nb_vues < 64)
                vues[nb_vues++] = p;
            ++s.echantillons;
        }
    }
    u64 const moyenne = total / s.echantillons;
    s.luminance_moyenne = static_cast<u32>(moyenne);
    s.variance = static_cast<u32>(carres / s.echantillons - moyenne * moyenne);
    s.couleurs = nb_vues;
    s.non_blanc_pct = non_blanc * 100 / s.echantillons;
    return s;
}

void BrowserWindow::present(BouchaudWebView& vue, NonnullRefPtr<Gfx::Bitmap> bitmap, int largeur, int hauteur, Gfx::IntRect degat)
{
    auto const debut = MonotonicTime::now();
    if (m_onglets_repris.remove(vue.onglet())) {
        auto const s = sonde_pixels(*bitmap, largeur, hauteur);
        warnln("[LB] PRESENT_APRES_REPRISE onglet={} page={} webcontent_pid={} taille={}x{} somme={:08x} variance={} couleurs={} non_blanc_pct={}",
            vue.onglet(), vue.page_courante(), vue.client().pid(), largeur, hauteur, s.somme, s.variance, s.couleurs, s.non_blanc_pct);
    }
    if (m_trames < 64 || m_trames % 16 == 15) {
        auto const s = sonde_pixels(*bitmap, largeur, hauteur);
        m_derniere_sonde = ByteString::formatted("onglet={} page={} seq={} damage={},{},{}x{} taille={}x{} somme={:08x} luminance={} variance={} couleurs={} non_blanc_pct={} echantillons={}",
            vue.onglet(), vue.page_courante(), m_trames + 1, degat.x(), degat.y(), degat.width(), degat.height(),
            largeur, hauteur, s.somme, s.luminance_moyenne, s.variance, s.couleurs, s.non_blanc_pct, s.echantillons);
        warnln("[LB] PRESENT {}", m_derniere_sonde);
    }
    if (BouchaudChrome::enabled())
        BouchaudChrome::present(vue.onglet(), move(bitmap), largeur, hauteur, { degat.x(), degat.y(), degat.width(), degat.height() });
    auto const duree_us = static_cast<u64>((MonotonicTime::now() - debut).to_microseconds());

    ++m_trames;
    m_present_us_total += duree_us;
    m_present_us_pire = max(m_present_us_pire, duree_us);

    if (!m_premiere_trame_vue) {
        m_premiere_trame_vue = true;
        warnln("BOUCHAUD_UI_FIRST_FRAME onglet={} page={} zone={}x{} source=compositor", vue.onglet(), vue.page_courante(), largeur, hauteur);
    }
    // La presentation est commandee par le DEGAT : une page immobile n'en
    // produit pas. Le plafond n'est atteint que par une page animee, et il est
    // la pour qu'elle ne noie pas le journal serie.
    //
    // `degat` : ce que le Compositor a declare change (repere de la page).
    // `vue` : la taille peinte. `publie` : le rectangle annonce au WM par
    // `FrameReady` (repere de la surface, barre d'outils comprise ; vide si la
    // trame n'a rien change a l'ecran). `copie_px` : les pixels reecrits.
    if (m_trames <= trames_journalisees || m_trames % 64 == 0) {
        auto const& c = BouchaudChrome::state();
        auto const& p = c.derniere_publication;
        warnln("[LB:FRAME] onglet={} seq={} t={} degat={},{} {}x{} vue={}x{} publie={},{} {}x{} copie_px={} complet={} present_us={}",
            vue.onglet(), m_trames, MonotonicTime::now().milliseconds(),
            degat.x(), degat.y(), degat.width(), degat.height(), largeur, hauteur,
            p.x, p.y, p.w, p.h, c.derniere_copie_px, c.derniere_complete ? 1 : 0, duree_us);
    }
    publie_compteurs_si_du();
}

void BrowserWindow::publie_compteurs_si_du()
{
    // Un releve a 16 et 64 trames, puis toutes les `releve_toutes_les` : une
    // page de banc n'en produit que quelques dizaines, et un releve qui
    // n'arrive qu'a la 256e ne serait jamais lu par le smoke.
    bool const releve_precoce = m_trames == 16 || m_trames == 64;
    if (!releve_precoce && m_trames - m_trames_publiees_au_releve < releve_toutes_les)
        return;
    m_trames_publiees_au_releve = m_trames;

    u64 degat = 0;
    u64 zone = 0;
    u64 sans_image = 0;
    for (auto const& vue : m_vues) {
        degat += vue->compteurs().degat_pixels;
        zone += vue->compteurs().zone_pixels;
        sans_image += vue->compteurs().trames_sans_image;
    }
    auto const& c = BouchaudChrome::state();
    // `degat/zone` : la part de la page que le Compositor a declaree changee.
    // `copie` : ce que le chrome a reellement ecrit dans la surface. Une copie
    // proche de la zone a chaque trame serait le symptome d'un degat perdu.
    warnln("[LB:PERF] trames={} vues={} degat_px={} zone_px={} copie_px={} trames_partielles={} trames_completes={} sans_effet={} sans_image={} present_us_moy={} present_us_pire={}",
        m_trames, m_vues.size(), degat, zone, c.chrome_pixels_written, c.chrome_partial_frames,
        c.chrome_full_frames, c.page_frames_sans_effet, sans_image,
        m_present_us_total / max<u64>(1, m_trames), m_present_us_pire);
}

static void imprime_console(u64 onglet, WebView::ConsoleOutput const& sortie)
{
    sortie.output.visit(
        [&](WebView::ConsoleLog const& journal) {
            StringBuilder texte;
            for (auto const& argument : journal.arguments) {
                if (!texte.is_empty())
                    texte.append(' ');
                if (argument.is_string()) {
                    texte.append(argument.as_string().bytes_as_string_view());
                } else {
                    auto serialise = argument.serialized();
                    texte.append(serialise.bytes_as_string_view());
                }
            }
            StringView niveau;
            switch (journal.level) {
            case JS::Console::LogLevel::Debug:
                niveau = "debug"sv;
                break;
            case JS::Console::LogLevel::Error:
                niveau = "erreur"sv;
                break;
            case JS::Console::LogLevel::Info:
                niveau = "info"sv;
                break;
            case JS::Console::LogLevel::Log:
                niveau = "log"sv;
                break;
            case JS::Console::LogLevel::Warn:
                niveau = "avert"sv;
                break;
            default:
                niveau = "autre"sv;
                break;
            }
            outln("[LB:JS] onglet={} {} {}", onglet, niveau, texte.string_view());
        },
        [&](WebView::ConsoleError const& erreur) {
            outln("[LB:JS] onglet={} erreur {} : {}", onglet, erreur.name, erreur.message);
            for (auto const& cadre : erreur.trace) {
                outln("[LB:JS] onglet={} cadre {} {}:{}", onglet,
                    cadre.function.has_value() ? cadre.function->bytes_as_string_view() : "(anonyme)"sv,
                    cadre.file.has_value() ? cadre.file->bytes_as_string_view() : "(inconnu)"sv,
                    cadre.line.value_or(0));
            }
        },
        [&](WebView::ConsoleTrace const& trace) {
            outln("[LB:JS] onglet={} trace {}", onglet, trace.label);
        });
}

void BrowserWindow::branche_vue(BouchaudWebView& vue)
{
    auto const onglet = vue.onglet();

    // BOUCHAUD_COMPOSITOR_LIEN_V1 -- banc de non-regression du crash
    // `ConnectionFromClient.cpp:68 VERIFICATION FAILED: connection`. Quand le
    // smoke exporte BOUCHAUD_LB_BANC_COUPE_LIEN, une page qui prend le titre
    // BOUCHAUD_BANC_COUPE_LIEN fait fermer, par SON WebContent, le lien vers
    // le Compositor (prepare-compositor-lien.py) : l'etat exact du journal
    // d'origine. Hors banc, un titre n'a aucun effet.
    static bool const banc_coupe_lien = getenv("BOUCHAUD_LB_BANC_COUPE_LIEN") != nullptr;
    // BOUCHAUD_CACHE_REDEMARRAGE_V1 -- banc du cache et de la base SQL a
    // travers un redemarrage COMPLET du navigateur : la page de banc prend le
    // titre BOUCHAUD_BANC_QUITTE quand elle a fini, et le navigateur quitte
    // proprement (boucle d'evenements, puis services). Hors banc
    // (BOUCHAUD_LB_BANC_QUITTE absent), un titre n'a aucun effet.
    static bool const banc_quitte = getenv("BOUCHAUD_LB_BANC_QUITTE") != nullptr;
    // BOUCHAUD_CRASH_RENDU_V1 -- banc d'isolation : une page qui prend le
    // titre BOUCHAUD_BANC_CRASH_RENDU fait fauter SON WebContent
    // (prepare-compositor-lien.py). Hors banc, un titre n'a aucun effet.
    static bool const banc_crash_rendu = getenv("BOUCHAUD_LB_BANC_CRASH_RENDU") != nullptr;
    // Banc des sites reels : quitter apres BOUCHAUD_LB_BANC_DUREE_S secondes,
    // en rejouant la derniere sonde de pixels (la trame finale de la page).
    if (auto const* duree = getenv("BOUCHAUD_LB_BANC_DUREE_S"); duree && !m_quitte_apres) {
        auto const secondes = max(5, atoi(duree));
        m_quitte_apres = Core::Timer::create_single_shot(secondes * 1000, [this, secondes] {
            warnln("[LB] PRESENT_DERNIER {}", m_derniere_sonde.is_empty() ? ByteString("aucune_trame") : m_derniere_sonde);
            warnln("[LB] BROWSER_QUIT_REQUEST raison=duree_s={}", secondes);
            Core::EventLoop::current().quit(0);
        });
        m_quitte_apres->start();
    }
    vue.on_title_change = [onglet, vue_ptr = &vue](Utf16String const& titre) {
        auto texte = titre.to_byte_string();
        if (banc_coupe_lien && texte == "BOUCHAUD_BANC_COUPE_LIEN"sv) {
            warnln("[LB] LINK_CUT_REQUEST onglet={} page={} t_ms={}", onglet, vue_ptr->page_courante(),
                MonotonicTime::now().milliseconds());
            vue_ptr->debug_request("bouchaud-coupe-lien-compositor"sv);
        }
        if (banc_crash_rendu && texte == "BOUCHAUD_BANC_CRASH_RENDU"sv) {
            warnln("[LB] RENDERER_CRASH_REQUEST onglet={} page={} webcontent_pid={} t_ms={}", onglet, vue_ptr->page_courante(),
                vue_ptr->client().pid(), MonotonicTime::now().milliseconds());
            vue_ptr->debug_request("bouchaud-crash-rendu"sv);
        }
        if (banc_quitte && texte == "BOUCHAUD_BANC_QUITTE"sv) {
            warnln("[LB] BROWSER_QUIT_REQUEST onglet={} raison=banc", onglet);
            Core::deferred_invoke([] { Core::EventLoop::current().quit(0); });
        }
        BouchaudChrome::set_title(onglet, texte);
    };
    vue.on_url_change = [onglet](URL::URL const& url) {
        BouchaudChrome::set_committed_url(onglet, url.serialize().to_byte_string());
    };
    vue.on_load_start = [onglet] {
        BouchaudChrome::set_loading(onglet, true, "chargement..."sv);
    };
    vue.on_load_finish = [onglet](URL::URL const& url) {
        auto charge = url.serialize().to_byte_string();
        // Le `about:blank` initial se termine pendant que la vraie navigation
        // est encore en vol. L'afficher ferait clignoter une URL que
        // l'utilisateur n'a pas demandee, juste avant la sienne.
        if (charge == "about:blank") {
            warnln("[LB:NAV] onglet={} document_ignore url=about:blank", onglet);
            return;
        }
        BouchaudChrome::set_committed_url(onglet, charge);
        BouchaudChrome::set_loading(onglet, false, "pret"sv);
        warnln("[LB:NAV] onglet={} document_charge url={}", onglet, charge);
    };
    vue.on_link_hover = [&vue](URL::URL const& url) {
        auto lien = url.serialize().to_byte_string();
        vue.set_lien_survole(lien);
        if (vue.onglet() == BouchaudChrome::page_active())
            BouchaudChrome::set_survol_url(lien);
    };
    vue.on_link_unhover = [&vue] {
        vue.set_lien_survole({});
        if (vue.onglet() == BouchaudChrome::page_active())
            BouchaudChrome::clear_survol_url();
    };
    vue.on_find_in_page = [onglet](size_t courant, Optional<size_t> const& total) {
        if (onglet != BouchaudChrome::page_active())
            return;
        BouchaudChrome::set_resultat_recherche(courant, total.has_value(), total.value_or(0));
    };
    vue.on_console_message = [onglet](WebView::ConsoleOutput sortie) {
        imprime_console(onglet, sortie);
    };

    // Menus contextuels : c'est LIBWEB qui les demande, apres avoir distribue
    // `contextmenu` au document -- une page qui l'annule garde le sien. Le
    // chrome les dessine ; la position arrive en coordonnees de PAGE.
    auto ouvre_menu = [&vue](Gfx::IntPoint position, bool avec_lien) {
        if (vue.onglet() != BouchaudChrome::page_active())
            return;
        BouchaudChrome::ouvre_menu_contextuel(position.x(), position.y(), avec_lien ? vue.lien_survole() : ByteString {});
    };
    vue.page_context_menu().on_activation = [ouvre_menu](Gfx::IntPoint position) { ouvre_menu(position, false); };
    vue.link_context_menu().on_activation = [ouvre_menu](Gfx::IntPoint position) { ouvre_menu(position, true); };
    vue.selected_text_link_context_menu().on_activation = [ouvre_menu](Gfx::IntPoint position) { ouvre_menu(position, true); };
    vue.image_context_menu().on_activation = [ouvre_menu](Gfx::IntPoint position) { ouvre_menu(position, true); };

    // Les boites de dialogue ne sont pas encore dessinees par le chrome. Une
    // boite laissee ouverte bloquerait le script de la page pour toujours :
    // elle est donc fermee tout de suite, et le journal le dit.
    vue.on_request_alert = [&vue](Utf16String const& message) {
        warnln("[LB:DIALOG] onglet={} alert ferme_auto=1 message={}", vue.onglet(), message);
        vue.alert_closed();
    };
    vue.on_request_confirm = [&vue](Utf16String const& message) {
        warnln("[LB:DIALOG] onglet={} confirm reponse=non message={}", vue.onglet(), message);
        vue.confirm_closed(false);
    };
    vue.on_request_prompt = [&vue](Utf16String const& message, Utf16String const&) {
        warnln("[LB:DIALOG] onglet={} prompt reponse=vide message={}", vue.onglet(), message);
        vue.prompt_closed({});
    };

    // Appele APRES la reprise d'upstream (handle_web_content_process_crash :
    // nouveau WebContent, page d'erreur, au plus 5 plantages rapproches).
    vue.on_web_content_crashed = [this, onglet, vue_ptr = &vue] {
        m_onglets_repris.set(onglet);
        warnln("[LB:CRASH] onglet={} webcontent=mort nouveau_pid={} t_ms={}", onglet, vue_ptr->client().pid(),
            MonotonicTime::now().milliseconds());
        BouchaudChrome::set_loading(onglet, false, "moteur arrete"sv);
    };

    vue.on_close = [this, onglet] {
        // `on_close` est appele depuis la vue elle-meme : la detruire ici
        // reviendrait a scier la branche. La boucle d'evenements le fera.
        Core::deferred_invoke([this, onglet] { retire_vue(onglet); });
    };
}

void BrowserWindow::branche_chrome()
{
    auto& c = BouchaudChrome::state();

    c.on_mouse_event = [this](Web::MouseEvent evenement) {
        if (auto* vue = vue_active())
            vue->enqueue_input_event(move(evenement));
    };
    c.on_key_event = [this](Web::KeyEvent evenement) {
        if (auto* vue = vue_active())
            vue->enqueue_input_event(move(evenement));
    };
    c.on_navigate = [this](ByteString cible) {
        auto* vue = vue_active();
        if (!vue)
            return;
        auto url = URL::create_with_url_or_path(cible);
        if (!url.has_value()) {
            warnln("[LB:NAV] url_invalide valeur=<{}>", cible);
            BouchaudChrome::set_loading(vue->onglet(), false, "URL invalide"sv);
            return;
        }
        warnln("[LB:NAV] onglet={} demande url={}", vue->onglet(), *url);
        vue->load(*url);
    };
    c.on_history_delta = [this](int delta) {
        if (auto* vue = vue_active())
            (void)vue->traverse_the_history_by_delta(delta);
    };
    c.on_reload = [this] {
        if (auto* vue = vue_active())
            vue->reload();
    };
    c.on_stop = [this] {
        if (auto* vue = vue_active())
            vue->stop_loading();
    };
    // Rien a redemander au moteur : la derniere trame presentee est la, il
    // suffit de la recopier en entier.
    c.on_repaint = [] {
        BouchaudChrome::compose_full();
    };
    c.on_resize = [this](int largeur, int hauteur) {
        if (largeur <= 0 || hauteur <= 0)
            return;
        m_viewport = Web::DevicePixelSize { largeur, hauteur };
        warnln("[LB:UI] viewport={}x{}", largeur, hauteur);
        for (auto& vue : m_vues)
            vue->reset_viewport_size(m_viewport);
    };
    c.on_zoom = [this](int pourcent) {
        if (auto* vue = vue_active())
            vue->set_zoom(static_cast<double>(pourcent) / 100.0);
    };
    c.on_nouvel_onglet = [this]() -> u64 {
        // Un onglet ouvert par l'utilisateur a son propre WebContent. Le
        // chrome l'enregistre lui-meme (`nouvel_onglet`) : ne pas l'annoncer.
        return ouvre_onglet(URL::about_blank(), true, false);
    };
    c.on_fermer_onglet = [this](u64 onglet) {
        // C'est le MOTEUR qui ferme : `beforeunload`, puis `on_close`.
        if (auto* vue = vue_de_l_onglet(onglet))
            vue->request_close();
    };
    c.on_onglet_actif = [this](u64 actif) {
        for (auto& vue : m_vues) {
            vue->set_system_visibility_state(vue->onglet() == actif
                    ? Web::HTML::VisibilityState::Visible
                    : Web::HTML::VisibilityState::Hidden);
        }
    };
    c.on_find = [this](ByteString requete) {
        if (auto* vue = vue_active())
            vue->find_in_page(Utf16String::from_utf8_with_replacement_character(requete));
    };
    c.on_find_next = [this] {
        if (auto* vue = vue_active())
            vue->find_in_page_next_match();
    };
    c.on_find_previous = [this] {
        if (auto* vue = vue_active())
            vue->find_in_page_previous_match();
    };
    c.on_select_all = [this] {
        if (auto* vue = vue_active())
            vue->select_all();
    };
    c.on_copy = [this]() -> ByteString {
        if (auto* vue = vue_active())
            return vue->selected_text();
        return {};
    };
    c.on_cut = [this]() -> ByteString {
        if (auto* vue = vue_active())
            return vue->cut_selected_text();
        return {};
    };
    // Le presse-papiers de l'Application EST celui du chrome
    // (`BouchaudUI::Application::clipboard_text`) : coller, c'est demander a
    // la vue de coller ce qu'il contient.
    c.on_paste = [this](ByteString) {
        if (auto* vue = vue_active())
            vue->paste_text_from_clipboard();
    };
    c.on_close = [] {
        warnln("[LB:UI] fermeture demandee");
        Core::EventLoop::current().quit(0);
    };
}

}
