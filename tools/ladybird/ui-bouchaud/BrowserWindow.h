/*
 * Bouchaud OS -- UI/Bouchaud : la fenetre du navigateur.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 *
 * BOUCHAUD_UI_V1
 *
 * La fenetre tient les onglets (une `BouchaudWebView` chacun) et le chrome
 * (`BouchaudChrome.h`). Elle est le SEUL lien entre les deux :
 *
 *   - le chrome demande (naviguer, recharger, ouvrir un onglet, entree
 *     clavier/souris...) par ses rappels, que la fenetre traduit en appels
 *     `ViewImplementation` upstream ;
 *   - les vues annoncent (titre, URL, chargement, trame presentee...), et la
 *     fenetre le reporte au chrome.
 *
 * Elle lit le canal GUI du gestionnaire de fenetres par un `Core::Notifier`
 * dans CE processus. Aucun WebContent ne voit ce canal.
 */

#pragma once

#include <AK/ByteString.h>
#include <AK/HashMap.h>
#include <AK/HashTable.h>
#include <AK/NonnullOwnPtr.h>
#include <AK/Time.h>
#include <AK/Vector.h>
#include <LibCore/AnonymousBuffer.h>
#include <LibCore/Forward.h>
#include <LibGfx/Forward.h>
#include <LibURL/URL.h>
#include <LibWeb/HTML/ActivateTab.h>
#include <LibWeb/PixelUnits.h>
#include <UI/Bouchaud/BouchaudWebView.h>

namespace BouchaudUI {

class BrowserWindow {
    AK_MAKE_NONCOPYABLE(BrowserWindow);
    AK_MAKE_NONMOVABLE(BrowserWindow);

public:
    BrowserWindow(Core::AnonymousBuffer theme, Web::DevicePixelSize viewport);
    ~BrowserWindow();

    /// Ouvre le premier onglet et commence a lire le canal GUI.
    void demarre(URL::URL const& url_initiale);

    /// Ouvre un onglet sur un WebContent neuf. Rend son identifiant.
    u64 ouvre_onglet(URL::URL const&, bool activer, bool annonce_au_chrome);

    /// `window.open`, `target=_blank` : la page demande une vue.
    String ouvre_vue_demandee_par_la_page(BouchaudWebView& ouvreur, Web::HTML::ActivateTab, Optional<u64> page_index);

    /// Une vue a presente une trame. Voir `BouchaudWebView::did_accept_presented_backing_store`.
    void present(BouchaudWebView&, NonnullRefPtr<Gfx::Bitmap>, int largeur, int hauteur, Gfx::IntRect degat);

    BouchaudWebView* vue_active() const;
    BouchaudWebView* vue_de_l_onglet(u64 onglet) const;
    size_t nombre_de_vues() const { return m_vues.size(); }

private:
    void branche_vue(BouchaudWebView&);
    void branche_chrome();
    void retire_vue(u64 onglet);
    void publie_compteurs_si_du();
    void demande_crash_service(u64 onglet, StringView service);
    void suit_crash_service();

    Core::AnonymousBuffer m_theme;
    Web::DevicePixelSize m_viewport;
    Vector<NonnullOwnPtr<BouchaudWebView>> m_vues;
    u64 m_prochain_onglet { 1 };

    RefPtr<Core::Notifier> m_notificateur_gui;
    RefPtr<Core::Timer> m_tic;

    u64 m_trames { 0 };
    u64 m_trames_publiees_au_releve { 0 };
    u64 m_present_us_total { 0 };
    u64 m_present_us_pire { 0 };
    bool m_premiere_trame_vue { false };

    // BOUCHAUD_SONDE_PIXELS_V1 : la derniere sonde, rejouee a la sortie de banc.
    ByteString m_derniere_sonde;
    // Onglets dont le WebContent vient d'etre remplace (plantage) : leur
    // premiere trame suivante est journalisee (PRESENT_APRES_REPRISE).
    HashTable<u64> m_onglets_repris;
    // Le WebContent courant de chaque onglet (PROCESS_SWAP : ancien -> nouveau).
    HashMap<u64, pid_t> m_webcontent_de_l_onglet;
    RefPtr<Core::Timer> m_quitte_apres;

    // Seulement pour les bancs : journal cumulatif, conserve apres
    // fermeture afin qu'une ligne serie perdue ne perde pas le process swap.
    struct PreuveSwap {
        pid_t ancien;
        pid_t nouveau;
        bool ferme { false };
        u64 changements { 1 };
    };
    HashMap<u64, PreuveSwap> m_preuves_swaps;
    RefPtr<Core::Timer> m_preuves_tic;
    u64 m_preuves_sequence { 0 };

    // BOUCHAUD_CRASH_SERVICES_V1 : le service tue par le banc, et son
    // remplacant (meme type, autre pid) tel que le gestionnaire le voit.
    struct SuiviCrashService {
        ByteString service;
        int type { -1 };
        pid_t ancien { 0 };
        pid_t nouveau { 0 };
        MonotonicTime depuis { MonotonicTime::now_coarse() };
        u64 trames_au_crash { 0 };
        u64 trames_au_remplacant { 0 };
        u32 tics { 0 };
        u32 tics_apres_remplacant { 0 };
    };
    SuiviCrashService m_suivi_crash;
    RefPtr<Core::Timer> m_suivi_crash_tic;
};

}
