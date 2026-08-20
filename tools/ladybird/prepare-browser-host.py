#!/usr/bin/env python3
"""Rendre a la page ses cookies, son stockage et son HSTS.

## Le defaut, et pourquoi il ne s'etait pas encore vu

Chez upstream, WebContent pose une dizaine de questions **synchrones** au
processus UI. Ce sont des questions d'etat, pas de rendu :

    DidRequestCookie          DidSetCookie          DidIsKnownHstsHost
    DidRequestStorageItem     DidSetStorageItem     DidRemoveStorageItem
    DidRequestStorageKeys     DidRequestStorageUsage    DidClearStorage
    StartWorkerAgent          DidRequestNewWebView      DidStartDownload

Bouchaud n'a pas de processus UI. Son lanceur tient la socket de controle mais
ne fait que la vider — personne n'y repond. Or `send_sync_but_allow_failure`
attend **sans delai d'expiration** : la reponse ne viendra jamais, et la
question ne rend jamais la main.

Le portage avait court-circuite les trois premieres avec un `return {}`. Cela
evitait le blocage, au prix d'une fonctionnalite : plus de cookies, plus de
HSTS. Les six suivantes n'etaient pas court-circuitees du tout — le premier
site qui touche `localStorage` fige donc WebContent pour toujours. Wikipedia et
Google en font tous deux usage.

## Ce que fait ce script

Il ne reimplemente rien. `WebView::CookieJar`, `WebView::StorageJar` et
`WebView::HSTSStore` sont des classes ordinaires de `LibWebView` — que
WebContent lie deja — et chacune offre une fabrique **en memoire** :

    static NonnullOwnPtr<CookieJar>   create(IsPrivate = IsPrivate::No);
    static NonnullOwnPtr<StorageJar>  create();
    static NonnullOwnPtr<HSTSStore>   create();

Le processus UI ne fait pas autre chose : ses gestionnaires sont des delegations
d'une ligne (`LibWebView/WebContentClient.cpp`). On les reproduit ici, dans
WebContent, en appelant les memes methodes des memes classes.

    upstream                                    Bouchaud
    --------------------------------------      -------------------------------
    Application::cookie_jar().get_cookie(...)   hote().cookies->get_cookie(...)
    Application::storage_jar().set_item(...)    hote().stockage->set_item(...)
    Application::hsts_store().is_known(...)     hote().hsts->is_known(...)

## Pourquoi un pot par processus est correct ici

Le portage n'a qu'un WebContent et lance `--site-isolation disable` : il n'y a
donc pas plusieurs moteurs a tenir coherents. Le jour ou Bouchaud aura un vrai
processus hote, ces trois pots devront y demenager sans changer de classe —
c'est precisement pour cela qu'on utilise celles d'upstream plutot que d'en
ecrire d'autres.

## Ce que cela ne couvre pas

`StartWorkerAgent`, `DidRequestNewWebView` et `DidStartDownload` restent des
questions sans repondant. Elles exigent un processus capable de **creer** un
autre processus ou une autre vue, ce qu'un pot en memoire ne remplace pas. Voir
docs/ladybird/AUDIT_INTEGRATION.md ; c'est la prochaine piece d'architecture.
"""
from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-browser-host.py <ladybird-worktree>")

root = Path(sys.argv[1])
page_cpp = root / "Services/WebContent/PageClient.cpp"
data = page_cpp.read_text()

if "bouchaud_hote_local" in data:
    print("hote local deja en place")
    raise SystemExit(0)


def substitute(old: str, new: str, label: str) -> None:
    global data
    if old not in data:
        raise SystemExit(f"hote local : ancre introuvable ({label})")
    data = data.replace(old, new, 1)


# ---------------------------------------------------------------------------
# Les inclusions et le porteur des trois pots.
# ---------------------------------------------------------------------------

substitute(
    "#include <WebContent/WebUIConnection.h>",
    """#include <WebContent/WebUIConnection.h>
#if defined(BOUCHAUD_PORT)
#    include <LibWebView/CookieJar.h>
#    include <LibWebView/HSTSStore.h>
#    include <LibWebView/StorageJar.h>
#endif""",
    "inclusions",
)

substitute(
    "namespace WebContent {\n",
    """namespace WebContent {

#if defined(BOUCHAUD_PORT)
// Voir tools/ladybird/prepare-browser-host.py.
//
// Les memes classes que celles du processus UI d'upstream, dans leur variante
// en memoire, tenues par le moteur faute d'hote a qui les confier. Alloue une
// fois, jamais detruit : leur duree de vie est celle du processus, et un
// destructeur de fin d'execution est une erreur de compilation chez Ladybird
// (-Wexit-time-destructors).
struct HoteLocal {
    NonnullOwnPtr<WebView::CookieJar> cookies;
    NonnullOwnPtr<WebView::StorageJar> stockage;
    NonnullOwnPtr<WebView::HSTSStore> hsts;
};

static HoteLocal& bouchaud_hote_local()
{
    static HoteLocal* the_host = new HoteLocal {
        WebView::CookieJar::create(WebView::IsPrivate::No),
        WebView::StorageJar::create(),
        WebView::HSTSStore::create(),
    };
    return *the_host;
}
#endif
""",
    "porteur",
)

# ---------------------------------------------------------------------------
# Cookies. Le portage rendait un cookie vide ; on rend le vrai.
# ---------------------------------------------------------------------------

substitute(
    """#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled())
        return {};
#endif
    auto response = client().send_sync_but_allow_failure<Messages::WebContentClient::DidRequestCookie>(m_id, url, source);""",
    """#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled()) {
        HTTP::Cookie::VersionedCookie cookie;
        cookie.cookie = bouchaud_hote_local().cookies->get_cookie(url, source);
        return cookie;
    }
#endif
    auto response = client().send_sync_but_allow_failure<Messages::WebContentClient::DidRequestCookie>(m_id, url, source);""",
    "lecture de cookie",
)

substitute(
    """#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled())
        return;
#endif
    auto response = client().send_sync_but_allow_failure<Messages::WebContentClient::DidSetCookie>(url, cookie, source);""",
    """#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled()) {
        bouchaud_hote_local().cookies->set_cookie(url, cookie, source);
        return;
    }
#endif
    auto response = client().send_sync_but_allow_failure<Messages::WebContentClient::DidSetCookie>(url, cookie, source);""",
    "ecriture de cookie",
)

substitute(
    """#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled())
        return false;
#endif
    auto response = client().send_sync_but_allow_failure<Messages::WebContentClient::DidIsKnownHstsHost>(domain);""",
    """#if defined(BOUCHAUD_PORT)
    // `is_known_hsts_host` prend une `StringView` et AK ne convertit pas
    // implicitement une `String` : la vue est explicite.
    if (bouchaud_m9_enabled())
        return bouchaud_hote_local().hsts->is_known_hsts_host(domain.bytes_as_string_view());
#endif
    auto response = client().send_sync_but_allow_failure<Messages::WebContentClient::DidIsKnownHstsHost>(domain);""",
    "interrogation HSTS",
)

substitute(
    """void PageClient::page_did_store_hsts_policy(String const& domain, HTTP::HSTS::ParsedHSTSPolicy const& policy)
{
    client().async_did_store_hsts_policy(domain, policy);""",
    """void PageClient::page_did_store_hsts_policy(String const& domain, HTTP::HSTS::ParsedHSTSPolicy const& policy)
{
#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled()) {
        bouchaud_hote_local().hsts->store_policy(domain, policy);
        return;
    }
#endif
    client().async_did_store_hsts_policy(domain, policy);""",
    "enregistrement HSTS",
)

# ---------------------------------------------------------------------------
# Stockage. Ces six-la n'etaient pas court-circuitees du tout : le premier
# `localStorage` figeait WebContent sans le moindre message.
# ---------------------------------------------------------------------------

stockage = [
    (
        "DidRequestStorageItem",
        "    auto response = client().send_sync_but_allow_failure<Messages::WebContentClient::DidRequestStorageItem>(storage_endpoint, storage_key, bottle_key);",
        "        return bouchaud_hote_local().stockage->get_item(storage_endpoint, storage_key, bottle_key);",
    ),
    (
        "DidSetStorageItem",
        "    auto response = client().send_sync_but_allow_failure<Messages::WebContentClient::DidSetStorageItem>(storage_endpoint, storage_key, bottle_key, value);",
        "        return bouchaud_hote_local().stockage->set_item(storage_endpoint, storage_key, bottle_key, value);",
    ),
    (
        "DidRemoveStorageItem",
        "    auto response = client().send_sync_but_allow_failure<Messages::WebContentClient::DidRemoveStorageItem>(storage_endpoint, storage_key, bottle_key);",
        "        bouchaud_hote_local().stockage->remove_item(storage_endpoint, storage_key, bottle_key);\n        return;",
    ),
    (
        "DidRequestStorageKeys",
        "    auto response = client().send_sync_but_allow_failure<Messages::WebContentClient::DidRequestStorageKeys>(storage_endpoint, storage_key);",
        "        return bouchaud_hote_local().stockage->get_all_keys(storage_endpoint, storage_key);",
    ),
    (
        "DidRequestStorageUsage",
        "    auto response = client().send_sync_but_allow_failure<Messages::WebContentClient::DidRequestStorageUsage>(storage_key);",
        "        return bouchaud_hote_local().stockage->usage(storage_key);",
    ),
    (
        "DidClearStorage",
        "    auto response = client().send_sync_but_allow_failure<Messages::WebContentClient::DidClearStorage>(storage_endpoint, storage_key);",
        "        bouchaud_hote_local().stockage->clear_storage_key(storage_endpoint, storage_key);\n        return;",
    ),
]

for nom, ancre, corps in stockage:
    substitute(
        ancre,
        "#if defined(BOUCHAUD_PORT)\n"
        "    if (bouchaud_m9_enabled()) {\n"
        f"{corps}\n"
        "    }\n"
        "#endif\n" + ancre,
        f"stockage {nom}",
    )

# ---------------------------------------------------------------------------
# Les trois questions qu'un pot en memoire ne peut pas remplacer.
#
# Elles demandent de **creer** quelque chose — un processus de worker, une
# nouvelle vue, un fichier telecharge — et cela exige un hote. Sans hote, elles
# attendent pour toujours une reponse qui ne viendra jamais, et la page se fige
# sans un mot.
#
# On decline donc, explicitement et bruyamment. Ce n'est pas un faux service :
# les trois valeurs rendues sont celles qu'upstream lui-meme produit quand le
# navigateur refuse. `page_did_request_new_web_view` a deja ce chemin
# (`if (!response->new_page_id().has_value()) return {};`) — c'est ce que fait
# un bloqueur de fenetres surgissantes. `page_did_start_download` rend un
# `Optional` vide, c'est-a-dire « aucun telechargement n'a commence ». Et un
# `WorkerAgentId` par defaut laisse `WorkerAgentParent` sans agent : le worker
# ne demarre pas, la page continue.
#
# Une limitation nommee vaut infiniment mieux qu'un gel. Voir
# docs/ladybird/AUDIT_INTEGRATION.md §5 pour la piece d'architecture qui les
# levera toutes les trois d'un coup.
# ---------------------------------------------------------------------------

refus = [
    (
        "worker",
        "    auto response = client().send_sync_but_allow_failure<Messages::WebContentClient::StartWorkerAgent>(m_id, move(request));",
        '        warnln("[ladybird-bouchaud] HOTE_ABSENT StartWorkerAgent : '
        'ce portage n\'a pas de processus hote, le worker ne demarrera pas");\n'
        "        return {};",
    ),
    (
        "nouvelle vue",
        "    auto response = client().send_sync_but_allow_failure<Messages::WebContentClient::DidRequestNewWebView>(m_id, activate_tab, hints);",
        '        warnln("[ladybird-bouchaud] HOTE_ABSENT DidRequestNewWebView : '
        'un seul onglet, la fenetre surgissante est refusee");\n'
        "        return {};",
    ),
    (
        "telechargement",
        "    auto response = client().send_sync<Messages::WebContentClient::DidStartDownload>(m_id, url, suggested_filename, total_size, request_server_client_id, request_server_request_id, move(initial_data));",
        '        warnln("[ladybird-bouchaud] HOTE_ABSENT DidStartDownload : '
        'aucun telechargement possible sans processus hote");\n'
        "        return {};",
    ),
    (
        "telechargement sans requete",
        "    auto response = client().send_sync<Messages::WebContentClient::DidStartDownloadWithoutRequest>(m_id, url, suggested_filename, total_size);",
        '        warnln("[ladybird-bouchaud] HOTE_ABSENT DidStartDownloadWithoutRequest : '
        'aucun telechargement possible sans processus hote");\n'
        "        return {};",
    ),
]

for nom, ancre, corps in refus:
    substitute(
        ancre,
        "#if defined(BOUCHAUD_PORT)\n"
        "    if (bouchaud_m9_enabled()) {\n"
        f"{corps}\n"
        "    }\n"
        "#endif\n" + ancre,
        f"refus {nom}",
    )

page_cpp.write_text(data)
print("Hote local (cookies, stockage, HSTS) branche dans", page_cpp)
