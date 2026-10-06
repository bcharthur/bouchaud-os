/*
 * Bouchaud OS -- UI/Bouchaud : une vue de page du navigateur.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Format.h>
#include <LibGfx/Bitmap.h>
#include <LibGfx/SharedImageBuffer.h>
#include <UI/Bouchaud/BouchaudWebView.h>
#include <UI/Bouchaud/BrowserWindow.h>

namespace BouchaudUI {

NonnullOwnPtr<BouchaudWebView> BouchaudWebView::create(BrowserWindow& window, u64 onglet, Core::AnonymousBuffer theme, Web::DevicePixelSize viewport)
{
    auto view = adopt_own(*new BouchaudWebView(window, onglet, move(theme), viewport));
    view->initialize_client(CreateNewClient::Yes);
    return view;
}

NonnullOwnPtr<BouchaudWebView> BouchaudWebView::create_child(BrowserWindow& window, u64 onglet, BouchaudWebView& parent, u64 page_index)
{
    auto view = adopt_own(*new BouchaudWebView(window, onglet, parent.m_theme, parent.m_viewport_size));
    view->m_client_state.client = parent.client();
    view->m_client_state.page_index = page_index;
    view->initialize_client(CreateNewClient::No);
    return view;
}

BouchaudWebView::BouchaudWebView(BrowserWindow& window, u64 onglet, Core::AnonymousBuffer theme, Web::DevicePixelSize viewport)
    : HeadlessWebView(move(theme), viewport)
    , m_window(window)
    , m_onglet(onglet)
{
    // `HeadlessWebView` fabrique d'autres `HeadlessWebView` pour les pages que
    // le document ouvre : elles n'auraient ni onglet ni fenetre. La fenetre en
    // fait des onglets.
    on_new_web_view = [this](Web::HTML::ActivateTab activate, Web::HTML::WebViewHints, Optional<u64> page_index) {
        return m_window.ouvre_vue_demandee_par_la_page(*this, activate, page_index);
    };
}

BouchaudWebView::~BouchaudWebView() = default;

// BOUCHAUD_UI_V1_PRESENTATION
//
// `ViewImplementation::server_did_paint` vient d'echanger le tampon de face :
// `front_bitmap` porte la trame que le Compositor a peinte, `damage_rect` ce
// qui a change depuis la trame precedente de CETTE vue. Le tampon reste celui
// de face jusqu'au prochain echange ; la fenetre le recopie avant de rendre la
// main, et ne retient qu'une reference qu'elle remplacera au prochain appel.
void BouchaudWebView::did_accept_presented_backing_store(i32 bitmap_id, Gfx::IntRect damage_rect)
{
    auto& front = m_client_state.front_bitmap;
    RefPtr<Gfx::Bitmap> bitmap;
    if (front.shared_image_buffer)
        bitmap = front.shared_image_buffer->bitmap_if_present();

    if (!bitmap) {
        // Un backing store sans pixels CPU : un tampon GPU partage. Le chemin
        // Bouchaud est `--force-cpu-painting` ; si ce cas arrive, il doit se
        // voir, pas se taire derriere une fenetre figee.
        if (m_compteurs.trames_sans_image++ == 0)
            warnln("[LB:FRAME] onglet={} bitmap={} sans_image_cpu=1", m_onglet, bitmap_id);
        return;
    }

    auto const largeur = front.last_painted_size.width().value();
    auto const hauteur = front.last_painted_size.height().value();
    ++m_compteurs.trames;
    m_compteurs.degat_pixels += static_cast<u64>(max(0, damage_rect.width())) * static_cast<u64>(max(0, damage_rect.height()));
    m_compteurs.zone_pixels += static_cast<u64>(max(0, largeur)) * static_cast<u64>(max(0, hauteur));

    m_window.present(*this, bitmap.release_nonnull(), largeur, hauteur, damage_rect);
}

}
