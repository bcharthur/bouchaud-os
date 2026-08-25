#!/usr/bin/env python3
"""A0 -- proprietaire unique du pont M11, et entree locale qui ne ment plus.

Le navigateur mourait sur une assertion d'AK, dans le processus HOTE :

    VERIFICATION FAILED: !is_empty() at AK/Queue.h:50

C'est `AK::Queue::dequeue()` appele sur une file vide, et la file est
`ViewImplementation::m_pending_input_events` (LibWebView/ViewImplementation.h).

L'INVARIANT VIOLE
-----------------
Cote hote, un evenement d'entree n'entre dans cette file qu'en un seul endroit,
`ViewImplementation::enqueue_input_event`, qui l'envoie ensuite a WebContent.
WebContent traite l'evenement puis rend UN accuse de reception :

    EventLoop.cpp   -> page_client.report_finished_handling_input_event(...)
    PageClient.cpp  -> client().async_did_finish_handling_input_event(...)
    hote            -> ViewImplementation::did_finish_handling_input_event
                       -> m_pending_input_events.dequeue()

Un accuse pour une mise en file. La file se vide donc exactement au rythme ou
l'hote la remplit -- tant que tous les evenements viennent de l'hote.

CE QUE LE PORTAGE FAISAIT
-------------------------
Le chrome M11 vit DANS WebContent et injecte l'entree localement :

    chrome.on_key_event   = [this, page_id](...) { key_event(page_id, ...); };
    chrome.on_mouse_event = [this, page_id](...) { mouse_event(page_id, ...); };

`key_event`/`mouse_event` sont les gestionnaires du point de terminaison IPC :
ils mettent l'evenement dans la file de WebContent, donc l'hote recoit un accuse
pour un evenement qu'il n'a JAMAIS envoye. Sa file est vide, `dequeue()` echoue,
l'assertion tombe et le processus meurt sur une instruction illegale.

Le premier evenement d'entree suffit. C'est pour cela que la CI ne l'a jamais
vu : elle tourne en `--headless=manual`, sans gestionnaire de fenetres et donc
sans le moindre evenement d'entree.

CE QUE FAIT CE SCRIPT
---------------------
1. `QueuedInputEvent` porte desormais `report_completion_to_client`. Un
   evenement injecte localement ne doit rendre aucun accuse : il n'y a rien a
   retirer chez l'hote. La boucle d'evenements respecte ce drapeau, y compris
   sur les evenements fusionnes et sur le chemin « page absente ».

   Le drapeau est pose par `enqueue_input_event` quand l'injection est en
   cours, ce qui permet de garder la FUSION des mouvements de souris
   d'upstream : on ne perd pas cette optimisation pour corriger le protocole.

   La fusion recoit en revanche une frontiere. `mouse_event`/`pinch_event`
   ecrivent directement dans `m_input_event_queue.tail()` et incrementent
   `coalesced_event_count` SANS repasser par `enqueue_input_event` : le
   drapeau du dernier element en file l'emporte alors sur celui de
   l'evenement entrant. Fusionner un evenement injecte dans un evenement de
   l'hote rendrait un accuse de trop -- l'assertion, de nouveau -- et la
   fusion inverse en retiendrait un, laissant `m_pending_input_events`
   grossir sans fin et l'entree de l'hote se bloquer. Les deux chemins de
   fusion refusent donc de traverser cette frontiere.

2. L'accuse coupe n'emporte plus la fin du traitement avec lui.

   Un seul appel portait deux choses : « l'hote peut depiler » et « le
   pipeline d'entree a fini cet evenement, avec ce resultat ». Couper le
   premier a fait disparaitre le second, et avec lui tout ce que le pont M11
   y avait accroche -- `WEB_WHEEL_HANDLED`, `WEB_SCREENSHOT_REQUEST`, et le
   readback du Compositor apres une molette
   (`prepare-full-browser-host.py`). Du code mort des que M11 est le seul a
   produire de l'entree, c'est-a-dire toujours : l'hote tourne en
   `--headless=manual`.

   Le port a donc un second point de sortie,
   `bouchaud_input_event_completed_locally`, qui dit la fin du traitement
   sans jamais toucher a la file de l'hote. `tools/ladybird/
   input-ownership-probe.cpp` verifie les deux moities : une notification
   locale par entree injectee mise en file, et une file de l'hote qui ne voit
   aucune difference selon que ce chemin existe ou non.

3. La propriete du pont M11 devient explicite. `initialize()` est appele dans
   CHAQUE processus WebContent, et `BO_GUI_FD`/`BO_SURFACE_FD` sont heritees
   par tout descendant : avec `site_isolation=top-level`, un second WebContent
   attachait un second chrome sur LES MEMES descripteurs. Le journal du
   25 aout le montre deux fois, avec des numeros identiques :

       M11_CHROME gui_fd=4 surface_fd=3   BROWSER_HOST_M11_ATTACHED page=1
       M11_CHROME gui_fd=4 surface_fd=3   BROWSER_HOST_M11_ATTACHED page=2

   Un seul proprietaire est admis par processus. Une seconde tentative est
   refusee et ANNONCEE, au lieu d'etre silencieusement doublee.
"""

import sys
from pathlib import Path

root = Path(sys.argv[1] if len(sys.argv) > 1 else ".")


def patch(path, old, new, quoi, obligatoire=True):
    fichier = root / path
    data = fichier.read_text()
    if new in data:
        return False
    if old not in data:
        if obligatoire:
            raise SystemExit(f"A0 : ancre introuvable ({quoi}) dans {path}")
        return False
    fichier.write_text(data.replace(old, new, 1))
    return True


def patch_tous(path, old, new, quoi, attendu):
    """Remplace TOUTES les occurrences, et refuse d'en trouver un autre nombre.

    Le portage est epingle sur un SHA precis : si upstream ajoute ou retire un
    de ces points de sortie, il vaut mieux que la preparation echoue bruyamment
    que de laisser un chemin non corrige rendre un accuse de trop.
    """
    fichier = root / path
    data = fichier.read_text()
    trouve = data.count(old)
    if trouve == 0:
        if data.count(new) >= attendu:
            return False
        raise SystemExit(f"A0 : ancre introuvable ({quoi}) dans {path}")
    if trouve != attendu:
        raise SystemExit(
            f"A0 : {trouve} occurrence(s) au lieu de {attendu} ({quoi}) dans {path}")
    fichier.write_text(data.replace(old, new))
    return True


# --- 1. Le drapeau sur l'evenement -----------------------------------------
patch(
    "Libraries/LibWeb/Page/InputEvent.h",
    """struct QueuedInputEvent {
    u64 page_id { 0 };
    InputEvent event;
    size_t coalesced_event_count { 0 };
};""",
    """struct QueuedInputEvent {
    u64 page_id { 0 };
    InputEvent event;
    size_t coalesced_event_count { 0 };

    // BOUCHAUD : un evenement injecte par le chrome M11, qui vit dans ce
    // processus, n'a pas d'entree correspondante dans la file d'attente de
    // l'hote. Lui rendre un accuse de reception ferait retirer a
    // `ViewImplementation::m_pending_input_events` un element qui n'y a
    // jamais ete mis -- c'est exactement l'assertion AK/Queue.h:50.
    bool report_completion_to_client { true };
};""",
    "QueuedInputEvent",
)

# --- 2. La boucle d'evenements respecte le drapeau --------------------------
#
# Le drapeau ne coupe que l'ACCUSE. Un seul appel portait deux choses
# distinctes, et les couper ensemble a fait disparaitre la seconde :
#
#   1. « l'hote peut depiler » -- ce qu'un evenement injecte ne doit surtout
#      pas dire, puisque l'hote n'a rien empile ;
#   2. « le pipeline d'entree a fini cet evenement, avec ce resultat » -- ce
#      dont le pont M11 a besoin pour TOUT evenement, injecte ou non, parce
#      que c'est la qu'il programme le readback du Compositor apres une
#      molette (`prepare-full-browser-host.py`).
#
# Le port a donc un second point de sortie, pour la notification purement
# locale. Sans lui, `WEB_WHEEL_HANDLED` et `WEB_SCREENSHOT_REQUEST` etaient du
# code mort des que M11 etait le seul a produire de l'entree -- c'est-a-dire
# toujours, l'hote tournant en `--headless=manual`.
patch(
    "Libraries/LibWeb/HTML/EventLoop/EventLoop.cpp",
    """            for (size_t i = 0; i < event.coalesced_event_count; ++i)
                page_client.report_finished_handling_input_event(event.page_id, EventResult::Dropped);
            page_client.did_handle_input_event(event.page_id, event.event);
            page_client.report_finished_handling_input_event(event.page_id, result);""",
    """            // BOUCHAUD : `did_handle_input_event` reste inconditionnel (il tient
            // l'etat de la methode de saisie), mais l'accuse de reception ne
            // part que si l'hote attend vraiment cet evenement. La fin du
            // traitement, elle, se dit dans les deux cas : voir
            // `bouchaud_input_event_completed_locally`.
            if (event.report_completion_to_client) {
                for (size_t i = 0; i < event.coalesced_event_count; ++i)
                    page_client.report_finished_handling_input_event(event.page_id, EventResult::Dropped);
            }
            page_client.did_handle_input_event(event.page_id, event.event);
            if (event.report_completion_to_client) {
                page_client.report_finished_handling_input_event(event.page_id, result);
            }
#if defined(BOUCHAUD_PORT)
            else {
                page_client.bouchaud_input_event_completed_locally(event.page_id, result);
            }
#endif""",
    "boucle d'evenements",
)

# --- 2b. Le point de sortie local ------------------------------------------
# Corps vide par defaut : aucune autre implementation de `Web::PageClient` n'a
# a le savoir, et rien ne change hors du port.
patch(
    "Libraries/LibWeb/Page/Page.h",
    """    virtual void report_finished_handling_input_event(u64 page_id, EventResult event_was_handled) = 0;""",
    """    virtual void report_finished_handling_input_event(u64 page_id, EventResult event_was_handled) = 0;

    // BOUCHAUD : l'entree injectee par le chrome M11 a fini son traitement.
    // L'hote ne doit pas en etre averti (il n'a rien empile), mais le port,
    // si : c'est la qu'il programme le readback du Compositor apres une
    // molette. Voir tools/ladybird/prepare-m11-input-ownership.py.
    virtual void bouchaud_input_event_completed_locally([[maybe_unused]] u64 page_id, [[maybe_unused]] EventResult event_was_handled) { }""",
    "sortie locale de fin d'entree",
)

patch(
    "Services/WebContent/PageClient.h",
    """    virtual void report_finished_handling_input_event(u64 page_id, Web::EventResult event_was_handled) override;""",
    """    virtual void report_finished_handling_input_event(u64 page_id, Web::EventResult event_was_handled) override;
#if defined(BOUCHAUD_PORT)
    virtual void bouchaud_input_event_completed_locally(u64 page_id, Web::EventResult event_was_handled) override;
#endif""",
    "declaration de la sortie locale",
)

# `report_finished_handling_input_event` a deja, sous BrowserHost+M11, une
# premiere branche qui rend AUCUN accuse : elle sort avant l'IPC. Ce qu'elle
# fait avant de sortir -- la sonde et le readback -- est exactement ce qu'un
# evenement injecte doit encore declencher. On l'y renvoie plutot que de
# recopier ce corps, sous la meme condition que sa sortie anticipee : ainsi
# les deux ne peuvent pas diverger et aucun accuse ne peut fuir.
patch(
    "Services/WebContent/PageClient.cpp",
    """void PageClient::report_finished_handling_input_event(u64 page_id, Web::EventResult event_was_handled)
{""",
    """#if defined(BOUCHAUD_PORT)
void PageClient::bouchaud_input_event_completed_locally(u64 page_id, Web::EventResult event_was_handled)
{
    // Meme condition que la sortie anticipee de
    // `report_finished_handling_input_event` : sous elle, cette fonction ne
    // touche pas a l'IPC et ne fait plus que le travail local. L'appeler ici
    // ne peut donc pas rendre l'accuse que ce chemin existe pour supprimer.
    if (getenv("BOUCHAUD_BROWSER_HOST") != nullptr && BouchaudChrome::enabled())
        report_finished_handling_input_event(page_id, event_was_handled);
}
#endif

void PageClient::report_finished_handling_input_event(u64 page_id, Web::EventResult event_was_handled)
{""",
    "implementation de la sortie locale",
)

# --- 3. L'injection locale, cote WebContent ---------------------------------
patch(
    "Services/WebContent/ConnectionFromClient.cpp",
    """void ConnectionFromClient::enqueue_input_event(Web::QueuedInputEvent event)
{
    auto page_id = event.page_id;
    auto page = m_page_host->page(page_id);
    if (!page.has_value()) {
        async_did_finish_handling_input_event(page_id, Web::EventResult::Dropped);
        return;
    }

    m_input_event_queue.enqueue(move(event));
    page->page().client().request_frame();
}""",
    """void ConnectionFromClient::enqueue_input_event(Web::QueuedInputEvent event)
{
    auto page_id = event.page_id;

    // BOUCHAUD : marque l'evenement AVANT toute sortie. Le chemin « page
    // absente » ci-dessous rend un accuse immediat ; sur un evenement injecte
    // il ferait tomber l'assertion aussi surement que le chemin nominal.
    if (m_bouchaud_local_input_depth > 0)
        event.report_completion_to_client = false;

    auto page = m_page_host->page(page_id);
    if (!page.has_value()) {
        bouchaud_report_input_dropped(page_id);
        return;
    }

    m_input_event_queue.enqueue(move(event));
    page->page().client().request_frame();
}

// BOUCHAUD : point de sortie unique pour l'accuse « evenement abandonne ». Les
// gestionnaires IPC le rendent sans passer par la file, y compris quand le
// `page_id` ne designe plus aucune page -- exactement le cas d'un chrome qui
// tape encore sur un identifiant devenu obsolete. Un evenement injecte n'a rien
// a accuser : l'hote ne l'a jamais mis dans `m_pending_input_events`.
void ConnectionFromClient::bouchaud_report_input_dropped(u64 page_id)
{
    if (m_bouchaud_local_input_depth > 0)
        return;
    async_did_finish_handling_input_event(page_id, Web::EventResult::Dropped);
}

// BOUCHAUD : entrees du chrome M11. Elles passent par `key_event`/`mouse_event`
// pour conserver la FUSION des mouvements de souris d'upstream ; seul l'accuse
// de reception est supprime, par le compteur ci-dessous.
void ConnectionFromClient::bouchaud_inject_key_event(u64 page_id, Web::KeyEvent event)
{
    ++m_bouchaud_local_input_depth;
    key_event(page_id, move(event));
    --m_bouchaud_local_input_depth;
}

void ConnectionFromClient::bouchaud_inject_mouse_event(u64 page_id, Web::MouseEvent event)
{
    ++m_bouchaud_local_input_depth;
    mouse_event(page_id, move(event));
    --m_bouchaud_local_input_depth;
}""",
    "enqueue_input_event",
)

# --- 4. Les sorties directes passent par le meme point ----------------------
# `mouse_event` et `pinch_event` rendent l'accuse eux-memes quand la page est
# absente, sans passer par `enqueue_input_event`. C'est le chemin qu'emprunte un
# chrome dont le `page_id` est devenu obsolete.
patch_tous(
    "Services/WebContent/ConnectionFromClient.cpp",
    "        async_did_finish_handling_input_event(page_id, Web::EventResult::Dropped);",
    "        bouchaud_report_input_dropped(page_id);",
    "sorties directes des gestionnaires d'entree",
    attendu=2,
)

# --- 5. La fusion ne traverse pas la frontiere de propriete -----------------
# `mouse_event` et `pinch_event` ecrivent dans l'element deja en file sans
# repasser par `enqueue_input_event`. C'est le drapeau de CET element qui
# decidera de l'accuse de reception, pas celui de l'evenement entrant : fusionner
# a travers la frontiere fausserait le compte dans un sens ou dans l'autre.
patch(
    "Services/WebContent/ConnectionFromClient.cpp",
    """        if (m_input_event_queue.is_empty())
            return nullptr;
        if (m_input_event_queue.tail().page_id != page_id)
            return nullptr;
""",
    """        if (m_input_event_queue.is_empty())
            return nullptr;
        if (m_input_event_queue.tail().page_id != page_id)
            return nullptr;

        // BOUCHAUD : ne jamais fusionner une entree injectee avec une entree
        // de l'hote. La fusion reecrit l'element en file et incremente
        // `coalesced_event_count` sans repasser par `enqueue_input_event` :
        // le compte d'accuses suivrait alors le drapeau du mauvais camp.
        if (m_input_event_queue.tail().report_completion_to_client != (m_bouchaud_local_input_depth == 0))
            return nullptr;
""",
    "frontiere de fusion souris",
)
patch(
    "Services/WebContent/ConnectionFromClient.cpp",
    "    if (!m_input_event_queue.is_empty() && m_input_event_queue.tail().page_id == page_id) {",
    """    if (!m_input_event_queue.is_empty() && m_input_event_queue.tail().page_id == page_id
        && m_input_event_queue.tail().report_completion_to_client == (m_bouchaud_local_input_depth == 0)) {""",
    "frontiere de fusion pincement",
)

# --- 6. Declarations ---------------------------------------------------------
patch(
    "Services/WebContent/ConnectionFromClient.h",
    "    void bouchaud_m11_start(u64 page_id);",
    """    void bouchaud_m11_start(u64 page_id);

    // BOUCHAUD : injection locale du chrome M11. Voir
    // tools/ladybird/prepare-m11-input-ownership.py.
    void bouchaud_inject_key_event(u64 page_id, Web::KeyEvent event);
    void bouchaud_inject_mouse_event(u64 page_id, Web::MouseEvent event);
    void bouchaud_report_input_dropped(u64 page_id);
    unsigned m_bouchaud_local_input_depth { 0 };""",
    "declarations d'injection",
)

# --- 7. Le chrome utilise l'injection ---------------------------------------
patch(
    "Services/WebContent/ConnectionFromClient.cpp",
    "        mouse_event(page_id, move(event));",
    "        bouchaud_inject_mouse_event(page_id, move(event));",
    "rappel souris du chrome",
)
patch(
    "Services/WebContent/ConnectionFromClient.cpp",
    "        key_event(page_id, move(event));",
    "        bouchaud_inject_key_event(page_id, move(event));",
    "rappel clavier du chrome",
)

# --- 8. Un seul proprietaire du pont M11 ------------------------------------
patch(
    "Services/WebContent/ConnectionFromClient.cpp",
    """        bouchaud_m11_start(initial_page_id);
        outln("[ladybird-bouchaud] BROWSER_HOST_M11_ATTACHED page={}", initial_page_id);""",
    """        // BOUCHAUD : un seul pont M11 par processus. `initialize()` est
        // appele dans CHAQUE WebContent, et les descripteurs du canal GUI et
        // de la surface sont heritees par tout descendant : sans ce verrou,
        // un second WebContent -- celui que l'isolation de site fait naitre --
        // attachait un second chrome sur les MEMES descripteurs.
        static u64 s_m11_owner_page = 0;
        static bool s_m11_owned = false;
        if (s_m11_owned) {
            outln("[ladybird-bouchaud] M11_OWNERSHIP_REFUSEE page={} proprietaire={}",
                initial_page_id, s_m11_owner_page);
        } else {
            s_m11_owned = true;
            s_m11_owner_page = initial_page_id;
            bouchaud_m11_start(initial_page_id);
            outln("[ladybird-bouchaud] BROWSER_HOST_M11_ATTACHED page={}", initial_page_id);
        }""",
    "propriete M11",
)

print("A0 applique au worktree:", root)
print(" - QueuedInputEvent::report_completion_to_client")
print(" - accuse de reception supprime pour l'entree injectee localement")
print(" - fusion d'entree interdite a travers la frontiere de propriete")
print(" - sorties « page absente » ramenees a un point unique")
print(" - un seul pont M11 par processus, refus annonce")
