/*
 * Bouchaud OS -- UI/Bouchaud : l'application navigateur.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/LexicalPath.h>
#include <LibCore/System.h>
#include <LibGfx/SystemTheme.h>
#include <LibWebView/Utilities.h>
#include <UI/Bouchaud/Application.h>
#include <UI/Bouchaud/BouchaudChrome.h>

#include <cerrno>
#include <fcntl.h>
#include <unistd.h>

namespace BouchaudUI {

Application::Application()
    : WebView::Application(ByteString { "/usr/libexec/ladybird" })
{
}

Application::~Application()
{
    // La fenetre tient des vues, et les vues des clients WebContent que
    // l'Application connait : elle part la premiere.
    m_fenetre = nullptr;
}

ErrorOr<void> Application::ouvre_la_fenetre(URL::URL const& url_initiale, Web::DevicePixelSize viewport)
{
    auto theme_path = LexicalPath::join(WebView::s_ladybird_resource_root, "themes"sv, "Default.ini"sv);
    auto theme = TRY(Gfx::load_system_theme(theme_path.string()));

    m_observateur_telechargements = make<ObservateurTelechargements>();
    m_fenetre = make<BrowserWindow>(move(theme), viewport);
    m_fenetre->demarre(url_initiale);
    return {};
}

Optional<String> Application::system_font_family() const
{
    // BOUCHAUD_POLICE_SYSTEME_V1
    //
    // Ladybird s'en sert pour la police par defaut, pour `system-ui`, et comme
    // dernier recours quand aucune famille demandee n'est trouvee -- c'est-a-
    // dire, en pratique, pour presque tout. DejaVu Sans est deposee par le
    // noyau dans /usr/share/fonts (`kernel::sysroot::install_fonts`) et couvre
    // le Latin accentue.
    return "DejaVu Sans"_string;
}

Optional<WebView::ViewImplementation&> Application::active_web_view() const
{
    if (m_fenetre) {
        if (auto* vue = m_fenetre->vue_active())
            return *vue;
    }
    return {};
}

Optional<WebView::ViewImplementation&> Application::open_blank_new_tab(Web::HTML::ActivateTab activer) const
{
    if (!m_fenetre)
        return {};
    auto onglet = m_fenetre->ouvre_onglet(URL::about_blank(), activer == Web::HTML::ActivateTab::Yes, true);
    if (auto* vue = m_fenetre->vue_de_l_onglet(onglet))
        return *vue;
    return {};
}

// ----------------------------------------------------------------------------
// Presse-papiers : celui du chrome, donc celui du bureau
// ----------------------------------------------------------------------------
//
// Le chrome tient la copie locale et la pousse au gestionnaire de fenetres
// (`Genre::PressePapiersEcrit`) ; le bureau la lui rend a la prise de foyer.
// L'Application n'en garde pas une seconde : deux presse-papiers dans un meme
// navigateur finissent toujours par diverger.

bool Application::supports_clipboard_type(ClipboardType type) const
{
    return type == ClipboardType::Text;
}

Utf16String Application::clipboard_text(ClipboardType) const
{
    // Le contenu vient du bureau, donc possiblement d'un autre programme :
    // `from_utf8` AFFIRMERAIT sa validite, et une affirmation sur une entree
    // etrangere est une panne qui attend.
    return Utf16String::from_utf8_with_replacement_character(BouchaudChrome::state().presse_papiers);
}

void Application::set_clipboard_text(String texte, ClipboardType)
{
    BouchaudChrome::copie_vers_le_presse_papiers(texte.to_byte_string());
}

Vector<Web::Clipboard::SystemClipboardRepresentation> Application::clipboard_entries() const
{
    auto const& texte = BouchaudChrome::state().presse_papiers;
    if (texte.is_empty())
        return {};
    return { { .data = texte, .mime_type = "text/plain"_string } };
}

void Application::insert_clipboard_item(Web::Clipboard::SystemClipboardItem item)
{
    for (auto& representation : item.system_clipboard_representations) {
        if (representation.mime_type == "text/plain"sv) {
            BouchaudChrome::set_presse_papiers_du_document(representation.data);
            return;
        }
    }
}

// ----------------------------------------------------------------------------
// Telechargements : ecrits par `WebView::FileDownloader`, montres par le chrome
// ----------------------------------------------------------------------------

ErrorOr<LexicalPath> Application::default_path_for_downloaded_file(ByteString const& file) const
{
    // Le nom vient du SERVEUR : assaini avant de toucher un chemin.
    auto const dossier = BouchaudChrome::dossier_de_telechargement();
    (void)Core::System::mkdir(dossier, 0755);
    auto const sur = BouchaudNomFichier::assainit(file.characters(), static_cast<int>(file.length()));
    StringView nom { sur.c_str(), static_cast<size_t>(sur.taille) };
    return LexicalPath { BouchaudChrome::chemin_de_telechargement(nom) };
}

Optional<ByteString> Application::ask_user_for_download_path(ByteString const& file) const
{
    // Pas de boite « Enregistrer sous » : le depot est fixe, comme en mode
    // headless upstream. Sans cette reponse, `path_for_downloaded_file`
    // annulerait chaque telechargement (ECANCELED).
    auto chemin = default_path_for_downloaded_file(file);
    if (chemin.is_error())
        return {};
    return chemin.value().string();
}

void Application::display_download_confirmation_dialog(StringView download_name, LexicalPath const& path) const
{
    warnln("[LB:DOWNLOAD] enregistre nom={} chemin={}", download_name, path);
}

void Application::display_error_dialog(StringView error_message) const
{
    warnln("[LB:UI] erreur {}", error_message);
}

static int etat_chrome(WebView::FileDownloader::DownloadStatus statut)
{
    switch (statut) {
    case WebView::FileDownloader::DownloadStatus::Completed:
        return 1;
    case WebView::FileDownloader::DownloadStatus::Canceled:
    case WebView::FileDownloader::DownloadStatus::Failed:
        return 2;
    case WebView::FileDownloader::DownloadStatus::InProgress:
    case WebView::FileDownloader::DownloadStatus::Paused:
        return 0;
    }
    VERIFY_NOT_REACHED();
}

// Un telechargement annonce comme termine doit survivre a une coupure qui
// arrive juste apres. `FileDownloader` ecrit un fichier temporaire puis le
// RENOMME a sa place ; il ne synchronise ni l'un ni l'autre. `/persist` est
// adosse au RAMFS : ce qui n'est pas `fsync` n'atteint le disque qu'a
// l'extinction. On synchronise donc le fichier final et son dossier (le
// renommage est une ecriture du dossier).
static void synchronise_fichier_termine(LexicalPath const& destination)
{
    for (auto const& chemin : { destination.string(), destination.dirname() }) {
        auto fd = Core::System::open(chemin, O_RDONLY | O_CLOEXEC);
        if (fd.is_error()) {
            warnln("[LB:DOWNLOAD] fsync impossible chemin={} erreur={}", chemin, fd.error());
            continue;
        }
        if (::fsync(fd.value()) != 0)
            warnln("[LB:DOWNLOAD] fsync echoue chemin={} errno={}", chemin, errno);
        (void)Core::System::close(fd.value());
    }
}

static void observe(WebView::FileDownloader::Download const& d)
{
    auto const etat = etat_chrome(d.status);
    if (etat == 1)
        synchronise_fichier_termine(d.destination);
    BouchaudChrome::observe_telechargement(d.id, ByteString { d.destination.basename() }, d.downloaded_size,
        d.total_size.has_value(), d.total_size.value_or(0), etat);
}

void Application::ObservateurTelechargements::download_added(WebView::FileDownloader::Download const& d)
{
    observe(d);
}

void Application::ObservateurTelechargements::download_updated(WebView::FileDownloader::Download const& d)
{
    observe(d);
}

}
