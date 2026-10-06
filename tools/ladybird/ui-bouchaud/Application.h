/*
 * Bouchaud OS -- UI/Bouchaud : l'application navigateur.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 *
 * BOUCHAUD_UI_V1
 *
 * `WebView::Application` upstream -- profil, magasins SQL, cookies, stockage,
 * HSTS, RequestServer, ImageDecoder, Compositor, WebWorker, FileDownloader --
 * plus ce qu'un frontend doit fournir : une fenetre, un presse-papiers, un
 * depot de telechargements. C'est le role de `UI/Qt/Application`, pour le
 * gestionnaire de fenetres de Bouchaud.
 */

#pragma once

#include <AK/OwnPtr.h>
#include <LibWebView/Application.h>
#include <LibWebView/FileDownloader.h>
#include <UI/Bouchaud/BrowserWindow.h>

namespace BouchaudUI {

class Application final : public WebView::Application {
    WEB_VIEW_APPLICATION(Application)

public:
    virtual ~Application() override;

    /// Cree la fenetre et ouvre le premier onglet. Appele apres `initialize`,
    /// donc apres que les services (RequestServer, Compositor...) sont la.
    ErrorOr<void> ouvre_la_fenetre(URL::URL const& url_initiale, Web::DevicePixelSize viewport);

    BrowserWindow* fenetre() const { return m_fenetre.ptr(); }

    virtual Optional<String> system_font_family() const override;

    virtual Optional<WebView::ViewImplementation&> active_web_view() const override;
    virtual Optional<WebView::ViewImplementation&> open_blank_new_tab(Web::HTML::ActivateTab) const override;

    virtual bool supports_clipboard_type(ClipboardType) const override;
    virtual Utf16String clipboard_text(ClipboardType = ClipboardType::Text) const override;
    virtual void set_clipboard_text(String, ClipboardType = ClipboardType::Text) override;
    virtual Vector<Web::Clipboard::SystemClipboardRepresentation> clipboard_entries() const override;
    virtual void insert_clipboard_item(Web::Clipboard::SystemClipboardItem) override;

    virtual ErrorOr<LexicalPath> default_path_for_downloaded_file(ByteString const& file) const override;
    virtual void display_download_confirmation_dialog(StringView download_name, LexicalPath const& path) const override;
    virtual void display_error_dialog(StringView error_message) const override;

protected:
    virtual bool should_coordinate_browser_process() const override { return false; }
    virtual Optional<ByteString> ask_user_for_download_path(ByteString const& file) const override;

private:
    Application();

    class ObservateurTelechargements final : public WebView::FileDownloaderObserver {
    public:
        virtual void download_added(WebView::FileDownloader::Download const&) override;
        virtual void download_updated(WebView::FileDownloader::Download const&) override;
    };

    OwnPtr<BrowserWindow> m_fenetre;
    OwnPtr<ObservateurTelechargements> m_observateur_telechargements;
};

}
