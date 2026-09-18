#!/usr/bin/env python3
"""Refresh DNS before requests; record startup milestones on stderr."""
from pathlib import Path
import sys

root = Path(sys.argv[1])
local = Path(__file__).resolve().parent

def insert(path, anchor, replacement, marker):
    data = path.read_text()
    if marker in data:
        return
    if data.count(anchor) != 1:
        raise SystemExit(f'Unexpected upstream anchor: {path}: {marker}')
    path.write_text(data.replace(anchor, replacement, 1))

client = root / 'Services/RequestServer/ConnectionFromClient.cpp'
(root / 'Services/RequestServer/BouchaudResolver.h').write_bytes(
    (local / 'network/BouchaudResolver.h').read_bytes())
insert(client, '#include <AK/IDAllocator.h>', '#include "BouchaudResolver.h"\n#include <cstdlib>\n#include <AK/IDAllocator.h>', '#include "BouchaudResolver.h"')
# Upgrade an already prepared V1 tree without stacking a second DNS gate.
data = client.read_text()
if 'BOUCHAUD_LIVE_DNS_V1' in data:
    data = data.replace('if (getenv("BOUCHAUD_BROWSER_HOST")) {',
        'if (getenv("BOUCHAUD_BROWSER_HOST") && (url.scheme() == "http"sv || url.scheme() == "https"sv)) {')
    client.write_text(data.replace('BOUCHAUD_LIVE_DNS_V1', 'BOUCHAUD_LIVE_DNS_V2'))
# Un arbre prepare en V2 porte deja la grille DNS mais pas le marqueur de
# navigation bloquee. `insert` s'arrete au premier marqueur trouve : sans ce
# rattrapage, la ligne qui porte l'URL ne serait jamais posee sur un arbre
# deja prepare.
data = client.read_text()
if 'BOUCHAUD_LIVE_DNS_V2' in data and 'BOUCHAUD_NAV_BLOQUEE' not in data:
    ancienne = '            warnln("[ladybird-bouchaud] BROWSER_NETWORK_NOT_READY id={}", request_id);'
    if data.count(ancienne) == 1:
        client.write_text(data.replace(
            ancienne,
            ancienne + '\n'
            '            warnln("[ladybird-bouchaud] BOUCHAUD_NAV_BLOQUEE url={} raison=network-not-ready", url.serialize());',
            1))

anchor = '    note_event_tick("ipc-start-request"sv);'
insert(client, anchor, '''#if defined(BOUCHAUD_PORT)
    // BOUCHAUD_LIVE_DNS_V2: envp is a launch-time snapshot; DHCP is not.
    if (getenv("BOUCHAUD_BROWSER_HOST") && (url.scheme() == "http"sv || url.scheme() == "https"sv)) {
        char dns[16] {};
        if (!BouchaudResolver::read(dns)) {
            warnln("[ladybird-bouchaud] BROWSER_NETWORK_NOT_READY id={}", request_id);
            // UNE NAVIGATION REFUSEE EST UNE NAVIGATION.
            //
            // Le noyau ne voit pas cette requete : elle est abandonnee AVANT
            // le moindre `connect`. Sans cette ligne, la fenetre Services
            // affiche « aucune navigation » au moment precis ou l'utilisateur
            // regarde une page qui ne charge pas. L'URL n'existe qu'ici.
            warnln("[ladybird-bouchaud] BOUCHAUD_NAV_BLOQUEE url={} raison=network-not-ready", url.serialize());
            async_request_finished(request_id, 0, {}, Requests::NetworkError::Unknown);
            return;
        }
        ByteString address { dns };
        if (address != DNSInfo::the().server_hostname) {
            set_dns_server(address, 53, false, false);
            warnln("[ladybird-bouchaud] BROWSER_DNS_UPDATED server={}", address);
        }
    }
#endif
''' + anchor, 'BOUCHAUD_NAV_BLOQUEE')

main = root / 'Services/WebContent/main.cpp'
for anchor, phase in [
    ('    auto& event_loop = Core::EventLoop::initialize_for_current_thread();', 'event-loop'),
    ('    font_provider.load_all_fonts_from_uri("resource://fonts"sv);', 'resource-fonts'),
    ('    font_provider.load_all_fonts_from_uri("file:///usr/share/fonts"sv);', 'system-fonts'),
    ('    Web::Platform::FontPlugin::install(*new Web::Platform::FontPlugin(enable_test_mode, &font_provider));', 'font-plugin'),
    ('    Web::Bindings::initialize_main_thread_vm(Web::HTML::AgentType::SimilarOriginWindow);', 'javascript-vm'),
]:
    marker = 'BROWSER_STARTUP_BEGIN phase=' + phase
    insert(main, anchor,
           '#if defined(BOUCHAUD_PORT)\n    warnln("' + marker + '");\n#endif\n' + anchor +
           '\n#if defined(BOUCHAUD_PORT)\n    warnln("BROWSER_STARTUP_END phase=' + phase + '");\n#endif', marker)
print('Live DNS and startup diagnostics applied')

request = root / 'Services/RequestServer/Request.cpp'
insert(request, 'static long s_connect_timeout_seconds = 90L;', '''#if defined(BOUCHAUD_PORT)
static long s_connect_timeout_seconds = 15L; // BOUCHAUD_CONNECT_DEADLINE_V1
#else
static long s_connect_timeout_seconds = 90L;
#endif''', 'BOUCHAUD_CONNECT_DEADLINE_V1')
insert(client, '    set_option(CURLMOPT_SOCKETFUNCTION, &on_socket_callback);', '''#if defined(BOUCHAUD_PORT)
    // BOUCHAUD_CONNECTION_BUDGET_V1: bound socket/handshake pressure per client.
    set_option(CURLMOPT_MAX_HOST_CONNECTIONS, 6L);
    set_option(CURLMOPT_MAX_TOTAL_CONNECTIONS, 16L);
#endif
    set_option(CURLMOPT_SOCKETFUNCTION, &on_socket_callback);''', 'BOUCHAUD_CONNECTION_BUDGET_V1')

# Route diagnostics only AFTER all structural patches consumed their anchors.
# Earlier routing broke input ownership and platform-complete preparation.
def route_diagnostics(root):
    for relative in ("Services/BouchaudBrowserHost/main.cpp",
                     "Services/WebContent/ConnectionFromClient.cpp"):
        path = root / relative
        data = path.read_text()
        path.write_text(data.replace('outln("[ladybird-bouchaud] BROWSER_HOST_',
                                     'warnln("[ladybird-bouchaud] BROWSER_HOST_'))

route_diagnostics(root)
