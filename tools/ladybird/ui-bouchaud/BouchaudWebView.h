/*
 * Bouchaud OS -- UI/Bouchaud : une vue de page du navigateur.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 *
 * BOUCHAUD_UI_V1
 *
 * Une `ViewImplementation` upstream, comme celles de `UI/Qt` et `UI/AppKit` :
 * elle parle a son WebContent par l'IPC normal (`WebContentClient`), recoit
 * les trames du Compositor, et n'a aucune connaissance du protocole GUI de
 * Bouchaud. Ce qui la distingue de `HeadlessWebView` tient en un point : la
 * trame presentee n'est pas jetee, elle est remise a la fenetre
 * (`BrowserWindow::present`) avec son rectangle de degat.
 */

#pragma once

#include <AK/NonnullOwnPtr.h>
#include <LibCore/AnonymousBuffer.h>
#include <LibWeb/PixelUnits.h>
#include <LibWebView/HeadlessWebView.h>

namespace BouchaudUI {

class BrowserWindow;

class BouchaudWebView final : public WebView::HeadlessWebView {
public:
    /// Une vue sur un WebContent NEUF : un onglet ouvert par l'utilisateur.
    static NonnullOwnPtr<BouchaudWebView> create(BrowserWindow&, u64 onglet, Core::AnonymousBuffer theme, Web::DevicePixelSize viewport);

    /// Une vue sur une page que le WebContent de `parent` a deja creee
    /// (`window.open`, `target=_blank`) : meme processus, autre page.
    static NonnullOwnPtr<BouchaudWebView> create_child(BrowserWindow&, u64 onglet, BouchaudWebView& parent, u64 page_index);

    virtual ~BouchaudWebView() override;

    /// Identifiant STABLE de l'onglet, attribue par la fenetre.
    ///
    /// Ce n'est pas `page_id()` : une navigation inter-sites peut faire
    /// changer la vue de WebContent, donc de page, et le chrome perdrait alors
    /// l'onglet qu'il affiche.
    u64 onglet() const { return m_onglet; }

    /// La page de WebContent que cette vue montre EN CE MOMENT -- pour les
    /// journaux. Elle change quand la vue change de processus.
    u64 page_courante() const { return page_id(); }

    /// Le lien sous le pointeur au dernier `on_link_hover`, vide sinon. C'est
    /// ce que le menu contextuel de lien propose de copier ou d'ouvrir.
    ByteString const& lien_survole() const { return m_lien_survole; }
    void set_lien_survole(ByteString lien) { m_lien_survole = move(lien); }

    struct Compteurs {
        u64 trames { 0 };
        u64 trames_sans_image { 0 };
        u64 degat_pixels { 0 };
        u64 zone_pixels { 0 };
    };
    Compteurs const& compteurs() const { return m_compteurs; }

private:
    BouchaudWebView(BrowserWindow&, u64 onglet, Core::AnonymousBuffer theme, Web::DevicePixelSize viewport);

    virtual void did_accept_presented_backing_store(i32 bitmap_id, Gfx::IntRect damage_rect) override;

    BrowserWindow& m_window;
    u64 m_onglet { 0 };
    ByteString m_lien_survole;
    Compteurs m_compteurs;
};

}
