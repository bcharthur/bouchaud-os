// Sonde du protocole d'accuse de reception des evenements d'entree (A0).
//
// POURQUOI ELLE EXISTE
// --------------------
// Le navigateur mourait sur `VERIFICATION FAILED: !is_empty() at AK/Queue.h:50`
// des le premier clic. La file en cause est `m_pending_input_events`, cote
// HOTE, et l'invariant viole tient en une phrase :
//
//     un accuse de reception rendu par WebContent pour UNE mise en file
//     faite par l'hote -- ni plus, ni moins.
//
// Le chrome M11 de Bouchaud vit DANS WebContent et injecte l'entree localement.
// Chaque evenement injecte produisait donc un accuse pour une mise en file qui
// n'avait jamais eu lieu : la file de l'hote se vidait sous elle-meme.
//
// CE QUE CETTE SONDE VERIFIE
// --------------------------
// Elle rejoue le protocole des deux cotes -- la file de l'hote, la file de
// WebContent, la fusion des mouvements, le chemin « page absente » -- et
// verifie les DEUX sens de l'invariant :
//
//   * jamais un accuse de trop  -> c'est l'assertion d'AK, le crash observe ;
//   * jamais un accuse de moins -> la file de l'hote grossirait sans fin et
//     son entree se bloquerait, panne silencieuse et non moins reelle.
//
// Elle rejoue chaque scenario DEUX fois : avec le portage d'avant A0 et avec
// celui d'apres. Le premier doit echouer, sinon la sonde ne prouve rien.
//
// CE QU'ELLE NE VERIFIE PAS
// -------------------------
// C'est un modele, pas le vrai code : elle ne compile pas LibWeb et ne peut
// donc pas attraper une faute de syntaxe ni une derive d'upstream. Le controle
// qui fait autorite reste la construction complete de Ladybird
// (`tools/ladybird/browser-upstream.sh`), puis un vrai clic dans QEMU.
//
// Compilation : c++ -std=c++17 -Wall -Wextra -Werror -o probe input-ownership-probe.cpp

#include <cstddef>
#include <cstdio>
#include <deque>
#include <set>
#include <string>
#include <vector>

namespace {

// --- Cote hote : LibWebView/ViewImplementation ------------------------------
//
// `enqueue_input_event` empile, `did_finish_handling_input_event` depile. Le
// depilement sur file vide est precisement l'assertion d'AK ; on l'enregistre
// au lieu de mourir, pour pouvoir l'affirmer dans un test.
struct Hote {
    long en_attente = 0;
    bool accuse_de_trop = false;

    void enqueue_input_event() { ++en_attente; }

    void did_finish_handling_input_event()
    {
        if (en_attente == 0) {
            accuse_de_trop = true; // AK/Queue.h:50
            return;
        }
        --en_attente;
    }
};

// --- Cote WebContent : Services/WebContent/ConnectionFromClient -------------
enum class Genre { Touche, Mouvement };

struct EvenementEnFile {
    unsigned page_id;
    Genre genre;
    std::size_t coalesced_event_count = 0;
    bool report_completion_to_client = true;
};

struct WebContent {
    Hote& hote;
    bool corrige;                     // false = portage d'avant A0
    std::set<unsigned> pages_vivantes;
    std::deque<EvenementEnFile> file;
    unsigned m_bouchaud_local_input_depth = 0;

    WebContent(Hote& h, bool c)
        : hote(h)
        , corrige(c)
    {
    }

    bool page_absente(unsigned page_id) const { return pages_vivantes.count(page_id) == 0; }

    // Point de sortie unique de l'accuse « evenement abandonne ».
    void bouchaud_report_input_dropped(unsigned)
    {
        if (corrige && m_bouchaud_local_input_depth > 0)
            return;
        hote.did_finish_handling_input_event();
    }

    void enqueue_input_event(EvenementEnFile event)
    {
        if (corrige && m_bouchaud_local_input_depth > 0)
            event.report_completion_to_client = false;

        if (page_absente(event.page_id)) {
            bouchaud_report_input_dropped(event.page_id);
            return;
        }
        file.push_back(event);
    }

    void key_event(unsigned page_id) { enqueue_input_event({ page_id, Genre::Touche, 0, true }); }

    void mouse_event(unsigned page_id)
    {
        if (page_absente(page_id)) {
            bouchaud_report_input_dropped(page_id);
            return;
        }

        // La fusion reecrit l'element DEJA en file sans repasser par
        // `enqueue_input_event` : c'est son drapeau a lui qui decidera de
        // l'accuse. Traverser la frontiere de propriete fausserait le compte.
        bool fusionnable = !file.empty()
            && file.back().page_id == page_id
            && file.back().genre == Genre::Mouvement;
        if (corrige && fusionnable)
            fusionnable = file.back().report_completion_to_client == (m_bouchaud_local_input_depth == 0);

        if (fusionnable) {
            ++file.back().coalesced_event_count;
            return;
        }
        enqueue_input_event({ page_id, Genre::Mouvement, 0, true });
    }

    void bouchaud_inject_key_event(unsigned page_id)
    {
        ++m_bouchaud_local_input_depth;
        key_event(page_id);
        --m_bouchaud_local_input_depth;
    }

    void bouchaud_inject_mouse_event(unsigned page_id)
    {
        ++m_bouchaud_local_input_depth;
        mouse_event(page_id);
        --m_bouchaud_local_input_depth;
    }

    // LibWeb/HTML/EventLoop : un accuse par evenement fusionne, puis un pour
    // l'evenement lui-meme -- seulement si l'hote attend vraiment celui-la.
    void boucle_evenements()
    {
        while (!file.empty()) {
            auto event = file.front();
            file.pop_front();
            if (corrige && !event.report_completion_to_client)
                continue;
            for (std::size_t i = 0; i < event.coalesced_event_count; ++i)
                hote.did_finish_handling_input_event();
            hote.did_finish_handling_input_event();
        }
    }
};

// L'entree qui vient de l'hote passe par IPC : elle est d'abord mise dans la
// file de l'hote, puis remise a WebContent.
void entree_hote_touche(WebContent& wc, unsigned page_id)
{
    wc.hote.enqueue_input_event();
    wc.key_event(page_id);
}

void entree_hote_mouvement(WebContent& wc, unsigned page_id)
{
    wc.hote.enqueue_input_event();
    wc.mouse_event(page_id);
}

// --- Les scenarios ----------------------------------------------------------
struct Resultat {
    bool accuse_de_trop;
    long reste_en_attente;
};

using Scenario = void (*)(WebContent&);

Resultat joue(Scenario scenario, bool corrige)
{
    Hote hote;
    WebContent wc(hote, corrige);
    wc.pages_vivantes = { 1, 2 };
    scenario(wc);
    wc.boucle_evenements();
    return { hote.accuse_de_trop, hote.en_attente };
}

void seulement_hote(WebContent& wc)
{
    for (int i = 0; i < 5; ++i)
        entree_hote_touche(wc, 1);
    for (int i = 0; i < 10; ++i)
        entree_hote_mouvement(wc, 1);
}

void seulement_chrome(WebContent& wc)
{
    for (int i = 0; i < 5; ++i)
        wc.bouchaud_inject_key_event(1);
    for (int i = 0; i < 10; ++i)
        wc.bouchaud_inject_mouse_event(1);
}

// Le cas reel : l'hote pousse l'entree du systeme, le chrome M11 pousse la
// sienne, et les mouvements des deux camps se suivent dans la meme file.
void entrelace(WebContent& wc)
{
    for (int i = 0; i < 8; ++i) {
        entree_hote_mouvement(wc, 1);
        wc.bouchaud_inject_mouse_event(1);
        entree_hote_touche(wc, 1);
        wc.bouchaud_inject_key_event(1);
    }
}

// Une seule injection suffisait a tuer le processus.
void un_seul_clic_du_chrome(WebContent& wc)
{
    wc.bouchaud_inject_mouse_event(1);
}

// Le `page_id` devenu obsolete : le chrome tape encore sur une page disparue.
// Le chemin « page absente » rend l'accuse sans passer par la file.
void page_obsolete(WebContent& wc)
{
    entree_hote_touche(wc, 1);
    wc.pages_vivantes.erase(1);
    wc.bouchaud_inject_key_event(1);
    wc.bouchaud_inject_mouse_event(1);
    entree_hote_touche(wc, 1); // l'hote, lui, doit toujours recevoir son accuse
}

// Vidange partielle : la boucle d'evenements tourne au milieu du flux, donc la
// fusion repart sur une file vide.
void vidange_au_milieu(WebContent& wc)
{
    entree_hote_mouvement(wc, 1);
    wc.bouchaud_inject_mouse_event(1);
    wc.boucle_evenements();
    wc.bouchaud_inject_mouse_event(1);
    entree_hote_mouvement(wc, 1);
}

// Deux pages : la fusion ne doit pas non plus traverser la frontiere de page.
void deux_pages(WebContent& wc)
{
    for (int i = 0; i < 4; ++i) {
        entree_hote_mouvement(wc, 1);
        entree_hote_mouvement(wc, 2);
        wc.bouchaud_inject_mouse_event(2);
    }
}

struct Cas {
    char const* nom;
    Scenario scenario;
    bool casse_avant_a0; // le portage d'avant doit echouer sur ce scenario
};

Cas const cas[] = {
    { "entree de l'hote seule", seulement_hote, false },
    { "entree du chrome seule", seulement_chrome, true },
    { "hote et chrome entrelaces", entrelace, true },
    { "un seul clic du chrome", un_seul_clic_du_chrome, true },
    { "page_id devenu obsolete", page_obsolete, true },
    { "vidange au milieu du flux", vidange_au_milieu, true },
    { "deux pages", deux_pages, true },
};

int echecs = 0;

void verifie(bool condition, std::string const& quoi)
{
    if (condition) {
        std::printf("  ok   %s\n", quoi.c_str());
        return;
    }
    std::printf("  ECHEC %s\n", quoi.c_str());
    ++echecs;
}

} // namespace

int main()
{
    std::printf("== apres A0 : l'invariant tient ==\n");
    for (auto const& c : cas) {
        auto r = joue(c.scenario, true);
        verifie(!r.accuse_de_trop, std::string(c.nom) + " : aucun accuse de trop");
        verifie(r.reste_en_attente == 0, std::string(c.nom) + " : aucun accuse manquant");
    }

    std::printf("\n== avant A0 : la sonde voit bien le defaut ==\n");
    for (auto const& c : cas) {
        if (!c.casse_avant_a0)
            continue;
        auto r = joue(c.scenario, false);
        verifie(r.accuse_de_trop || r.reste_en_attente != 0,
            std::string(c.nom) + " : le portage d'avant A0 rompt l'invariant");
    }

    std::printf("\n== avant A0 : le trafic purement hote etait deja correct ==\n");
    auto temoin = joue(seulement_hote, false);
    verifie(!temoin.accuse_de_trop && temoin.reste_en_attente == 0,
        "entree de l'hote seule : inchangee par A0");

    if (echecs != 0) {
        std::printf("\n%d verification(s) en echec\n", echecs);
        return 1;
    }
    std::printf("\ntout passe\n");
    return 0;
}
