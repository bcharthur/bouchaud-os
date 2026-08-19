#!/usr/bin/env python3
# Focused M9 diagnostics layered after prepare-m9-source.py.
# This file only touches the disposable pinned Ladybird worktree.

from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-m9-diagnostics.py <ladybird-worktree>")

root = Path(sys.argv[1])


def replace_once(path: Path, old: str, new: str, label: str):
    data = path.read_text()
    if new in data:
        return
    if old not in data:
        raise SystemExit(f"M9 diagnostics pattern not found ({label}) in {path}")
    path.write_text(data.replace(old, new, 1))


# ---------------------------------------------------------------------------
# LibRequests: distinguish four different states which previous markers could
# not separate:
#   1. request_finished IPC arrived;
#   2. response pipe was activated;
#   3. bytes were actually read and delivered;
#   4. the user's completion callback was finally released.
# ---------------------------------------------------------------------------
request_cpp = root / "Libraries/LibRequests/Request.cpp"
data = request_cpp.read_text()

if "M9_BODY_FINISH_SIGNAL" not in data:
    old = '''    on_finish = [this](auto total_size, auto const& timing_info, auto network_error) {
        // If the request was stopped while this IPC was in-flight, just bail.
        if (!m_internal_stream_data)
            return;

        m_internal_stream_data->total_size = total_size;
        m_internal_stream_data->network_error = network_error;
        m_internal_stream_data->timing_info = timing_info;
        m_internal_stream_data->request_done = true;
        m_internal_stream_data->on_finish();
    };'''
    new = '''    on_finish = [this](auto total_size, auto const& timing_info, auto network_error) {
        // If the request was stopped while this IPC was in-flight, just bail.
        if (!m_internal_stream_data)
            return;

#if defined(BOUCHAUD_PORT)
        if (getenv("BOUCHAUD_M9") != nullptr)
            outln("[ladybird-bouchaud] M9_BODY_FINISH_SIGNAL total={} delivered={}", total_size, m_internal_stream_data->delivered_size);
#endif
        m_internal_stream_data->total_size = total_size;
        m_internal_stream_data->network_error = network_error;
        m_internal_stream_data->timing_info = timing_info;
        m_internal_stream_data->request_done = true;
        m_internal_stream_data->on_finish();
    };'''
    if old not in data:
        raise SystemExit("M9 diagnostics: Request on_finish wrapper changed upstream")
    data = data.replace(old, new, 1)

if "M9_BODY_FINISH_GATE" not in data:
    old = '''        auto has_received_all_reported_bytes = m_internal_stream_data->request_done && m_internal_stream_data->delivered_size >= m_internal_stream_data->total_size;
        if (!m_internal_stream_data->user_finish_called && (!m_internal_stream_data->read_stream || m_internal_stream_data->read_stream->is_eof() || has_received_all_reported_bytes)) {
            m_internal_stream_data->user_finish_called = true;
            user_on_finish(m_internal_stream_data->total_size, m_internal_stream_data->timing_info, m_internal_stream_data->network_error);
        }'''
    new = '''        auto has_received_all_reported_bytes = m_internal_stream_data->request_done && m_internal_stream_data->delivered_size >= m_internal_stream_data->total_size;
#if defined(BOUCHAUD_PORT)
        if (getenv("BOUCHAUD_M9") != nullptr)
            outln("[ladybird-bouchaud] M9_BODY_FINISH_GATE done={} delivered={} total={} eof={} called={}", m_internal_stream_data->request_done, m_internal_stream_data->delivered_size, m_internal_stream_data->total_size, m_internal_stream_data->read_stream ? m_internal_stream_data->read_stream->is_eof() : true, m_internal_stream_data->user_finish_called);
#endif
        if (!m_internal_stream_data->user_finish_called && (!m_internal_stream_data->read_stream || m_internal_stream_data->read_stream->is_eof() || has_received_all_reported_bytes)) {
            m_internal_stream_data->user_finish_called = true;
#if defined(BOUCHAUD_PORT)
            if (getenv("BOUCHAUD_M9") != nullptr)
                outln("[ladybird-bouchaud] M9_BODY_USER_FINISH delivered={} total={}", m_internal_stream_data->delivered_size, m_internal_stream_data->total_size);
#endif
            user_on_finish(m_internal_stream_data->total_size, m_internal_stream_data->timing_info, m_internal_stream_data->network_error);
        }'''
    if old not in data:
        raise SystemExit("M9 diagnostics: Request finish gate changed upstream")
    data = data.replace(old, new, 1)

if "M9_BODY_READ_ERROR" not in data:
    old = '''            auto result = m_internal_stream_data->read_stream->read_some({ buffer, bytes_to_read });
            if (result.is_error() && (!result.error().is_errno() || (result.error().is_errno() && result.error().code() != EINTR)))
                break;
            if (result.is_error())
                continue;

            auto read_bytes = result.release_value();
            if (read_bytes.is_empty())
                break;

            m_internal_stream_data->delivered_size += read_bytes.size();
            m_internal_stream_data->on_data_available(ResponseData::from_bytes(read_bytes));'''
    new = '''            auto result = m_internal_stream_data->read_stream->read_some({ buffer, bytes_to_read });
            if (result.is_error() && (!result.error().is_errno() || (result.error().is_errno() && result.error().code() != EINTR))) {
#if defined(BOUCHAUD_PORT)
                if (getenv("BOUCHAUD_M9") != nullptr)
                    outln("[ladybird-bouchaud] M9_BODY_READ_ERROR code={} delivered={}", result.error().code(), m_internal_stream_data->delivered_size);
#endif
                break;
            }
            if (result.is_error())
                continue;

            auto read_bytes = result.release_value();
            if (read_bytes.is_empty()) {
#if defined(BOUCHAUD_PORT)
                if (getenv("BOUCHAUD_M9") != nullptr)
                    outln("[ladybird-bouchaud] M9_BODY_READ_EMPTY delivered={}", m_internal_stream_data->delivered_size);
#endif
                break;
            }

            m_internal_stream_data->delivered_size += read_bytes.size();
#if defined(BOUCHAUD_PORT)
            if (getenv("BOUCHAUD_M9") != nullptr)
                outln("[ladybird-bouchaud] M9_BODY_READ chunk={} delivered={}", read_bytes.size(), m_internal_stream_data->delivered_size);
#endif
            m_internal_stream_data->on_data_available(ResponseData::from_bytes(read_bytes));'''
    if old not in data:
        raise SystemExit("M9 diagnostics: Request read loop changed upstream")
    data = data.replace(old, new, 1)

request_cpp.write_text(data)


# ---------------------------------------------------------------------------
# PageClient: M9 deliberately has no Browser process. These three messages are
# DevTools/network-observability notifications only; ResourceLoader calls them
# before forwarding headers/completion into Fetch. Do not send them into the
# bootstrap socket in M9. This mirrors the existing M9 cookie/HSTS/local
# navigation policy and leaves every normal Ladybird build untouched.
# ---------------------------------------------------------------------------
page_cpp = root / "Services/WebContent/PageClient.cpp"
data = page_cpp.read_text()

patches = [
    (
        '''void PageClient::page_did_start_network_request(u64 request_id, URL::URL const& url, ByteString const& method, Vector<HTTP::Header> const& request_headers, ReadonlyBytes request_body, Optional<String> initiator_type, String const& referrer_policy, bool is_navigation_request, Web::Fetch::Infrastructure::Request::Priority priority)
{
    client().async_did_start_network_request(m_id, request_id, url, method, request_headers, request_body, move(initiator_type), referrer_policy, is_navigation_request, priority);
}''',
        '''void PageClient::page_did_start_network_request(u64 request_id, URL::URL const& url, ByteString const& method, Vector<HTTP::Header> const& request_headers, ReadonlyBytes request_body, Optional<String> initiator_type, String const& referrer_policy, bool is_navigation_request, Web::Fetch::Infrastructure::Request::Priority priority)
{
#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled()) {
        outln("[ladybird-bouchaud] M9_BROWSER_NET_START_SKIPPED id={} url={}", request_id, url);
        return;
    }
#endif
    client().async_did_start_network_request(m_id, request_id, url, method, request_headers, request_body, move(initiator_type), referrer_policy, is_navigation_request, priority);
}''',
        "PageClient network start",
    ),
    (
        '''void PageClient::page_did_receive_network_response_headers(u64 request_id, u32 status_code, Optional<String> reason_phrase, Vector<HTTP::Header> const& response_headers, Requests::CameFromCache came_from_cache)
{
    client().async_did_receive_network_response_headers(m_id, request_id, status_code, move(reason_phrase), response_headers, came_from_cache);
}''',
        '''void PageClient::page_did_receive_network_response_headers(u64 request_id, u32 status_code, Optional<String> reason_phrase, Vector<HTTP::Header> const& response_headers, Requests::CameFromCache came_from_cache)
{
#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled()) {
        outln("[ladybird-bouchaud] M9_BROWSER_NET_HEADERS_SKIPPED id={} status={}", request_id, status_code);
        return;
    }
#endif
    client().async_did_receive_network_response_headers(m_id, request_id, status_code, move(reason_phrase), response_headers, came_from_cache);
}''',
        "PageClient network headers",
    ),
    (
        '''void PageClient::page_did_finish_network_request(u64 request_id, u64 body_size, Requests::RequestTimingInfo const& timing_info, Optional<Requests::NetworkError> const& network_error)
{
    client().async_did_finish_network_request(m_id, request_id, body_size, timing_info, network_error);
}''',
        '''void PageClient::page_did_finish_network_request(u64 request_id, u64 body_size, Requests::RequestTimingInfo const& timing_info, Optional<Requests::NetworkError> const& network_error)
{
#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled()) {
        outln("[ladybird-bouchaud] M9_BROWSER_NET_FINISH_SKIPPED id={} size={} error={}", request_id, body_size, network_error.has_value());
        return;
    }
#endif
    client().async_did_finish_network_request(m_id, request_id, body_size, timing_info, network_error);
}''',
        "PageClient network finish",
    ),
]

for old, new, label in patches:
    if new in data:
        continue
    if old not in data:
        raise SystemExit(f"M9 diagnostics pattern not found ({label}) in {page_cpp}")
    data = data.replace(old, new, 1)

page_cpp.write_text(data)

# ---------------------------------------------------------------------------
# RequestServer : nommer l'etat ou la requete s'arrete.
#
# Le journal de l'essai Internet passe de `M9_BROWSER_NET_START_SKIPPED` — donc
# ResourceLoader a bien lance la requete — a un silence complet : aucun
# `M9_RS_REQUEST_STARTED`, aucun `M9_RS_HEADERS`, aucune erreur. Cote WebContent
# il n'y a plus rien a observer : tout ce qui manque se passe dans l'autre
# processus.
#
# Or RequestServer n'est pas une boite noire : `Request` est une machine a etats
# explicite (Init, ReadCache, WaitForCache, DNSLookup, RetrieveCookie, Connect,
# Fetch, Complete, Error) dont chaque passage traverse `transition_to_state`.
# Une seule sonde a cet endroit remplace le silence par le nom de l'etat
# atteint, et donc par un sous-systeme unique a examiner :
#
#   - arret en `DNSLookup`      -> LibDNS ou le resolveur ;
#   - arret en `RetrieveCookie` -> l'aller-retour IPC des cookies, qui passe par
#                                  `on_retrieve_http_cookie` — un rappel que
#                                  seule `LibWebView::Application` installe, et
#                                  que le portage M9 n'a pas ;
#   - arret en `Connect`/`Fetch`-> libcurl, TLS, ou les notifiers de socket.
#
# Upstream a deja la ligne qu'il faut, mais derriere `REQUESTSERVER_DEBUG`, un
# drapeau de compilation : l'activer recompilerait tout RequestServer en mode
# verbeux et noierait la console serie. La sonde reprend donc la meme
# information sous la variable d'environnement que le reste du portage utilise.
request_server_cpp = root / "Services/RequestServer/Request.cpp"
data = request_server_cpp.read_text()

if "M9_RS_STATE" not in data:
    # `getenv` n'est pas garanti par les inclusions de ce fichier ; l'ajouter
    # explicitement plutot que de dependre d'une inclusion transitive.
    if "<cstdlib>" not in data:
        premiere = data.index("#include ")
        data = data[:premiere] + "#include <cstdlib>\n" + data[premiere:]

    ancien = """void Request::transition_to_state(State state)
{
    dbgln_if(REQUESTSERVER_DEBUG, "Request::Transition[{}]: {} -> {} ({} {})", m_request_id, state_name(m_state), state_name(state), m_method, m_url);
    m_state = state;
    process();
}"""
    nouveau = """void Request::transition_to_state(State state)
{
    dbgln_if(REQUESTSERVER_DEBUG, "Request::Transition[{}]: {} -> {} ({} {})", m_request_id, state_name(m_state), state_name(state), m_method, m_url);
#if defined(BOUCHAUD_PORT)
    if (getenv("BOUCHAUD_M9") != nullptr)
        outln("[ladybird-bouchaud] M9_RS_STATE id={} {} -> {}", m_request_id, state_name(m_state), state_name(state));
#endif
    m_state = state;
    process();
}"""
    if ancien not in data:
        raise SystemExit("M9 diagnostics: Request::transition_to_state a change en amont")
    data = data.replace(ancien, nouveau, 1)

    # La transition ne dit pas *quel* nom est resolu ni en quelles adresses.
    # Sans cela, un arret en `DNSLookup` reste partage entre « la requete n'est
    # jamais partie » et « la reponse n'est jamais revenue ».
    ancien_dns = """    mark_lifecycle_event(this, &WireStats::dns_started_at);

    m_resolver->dns.lookup(host, DNS::Messages::Class::IN, { DNS::Messages::ResourceType::A, DNS::Messages::ResourceType::AAAA }, { .validate_dnssec_locally = dns_info.validate_dnssec_locally })"""
    nouveau_dns = """    mark_lifecycle_event(this, &WireStats::dns_started_at);

#if defined(BOUCHAUD_PORT)
    if (getenv("BOUCHAUD_M9") != nullptr)
        outln("[ladybird-bouchaud] M9_RS_DNS_QUERY id={} host={} serveur={}", m_request_id, host, dns_info.server_address.has_value() ? dns_info.server_address->to_byte_string() : ByteString { "(systeme)" });
#endif
    m_resolver->dns.lookup(host, DNS::Messages::Class::IN, { DNS::Messages::ResourceType::A, DNS::Messages::ResourceType::AAAA }, { .validate_dnssec_locally = dns_info.validate_dnssec_locally })"""
    if ancien_dns not in data:
        raise SystemExit("M9 diagnostics: handle_dns_lookup_state a change en amont")
    data = data.replace(ancien_dns, nouveau_dns, 1)

    ancien_resolu = """            } else if (first_is_one_of(self.m_type, RequestType::Fetch, RequestType::BackgroundRevalidation)) {
                self.m_dns_result = move(dns_result);
                self.transition_to_state(State::RetrieveCookie);"""
    nouveau_resolu = """            } else if (first_is_one_of(self.m_type, RequestType::Fetch, RequestType::BackgroundRevalidation)) {
#if defined(BOUCHAUD_PORT)
                if (getenv("BOUCHAUD_M9") != nullptr)
                    outln("[ladybird-bouchaud] M9_RS_DNS_RESOLU id={} host={} adresses={}", self.m_request_id, host, dns_result->cached_addresses().size());
#endif
                self.m_dns_result = move(dns_result);
                self.transition_to_state(State::RetrieveCookie);"""
    if ancien_resolu not in data:
        raise SystemExit("M9 diagnostics: branche DNS resolue a change en amont")
    data = data.replace(ancien_resolu, nouveau_resolu, 1)

    # `RetrieveCookie` est le seul etat qui rende la main a *l'autre* processus
    # avant meme d'avoir ouvert une socket. S'il est atteint, savoir si la
    # demande est partie separe « le client n'a jamais recu » de « le client n'a
    # jamais repondu ».
    ancien_cookie = """        connection->async_retrieve_http_cookie(m_client->client_id(), m_request_id, m_type, m_url, m_client->is_private());"""
    nouveau_cookie = """#if defined(BOUCHAUD_PORT)
        if (getenv("BOUCHAUD_M9") != nullptr)
            outln("[ladybird-bouchaud] M9_RS_COOKIE_DEMANDE id={}", m_request_id);
#endif
        connection->async_retrieve_http_cookie(m_client->client_id(), m_request_id, m_type, m_url, m_client->is_private());"""
    if ancien_cookie not in data:
        raise SystemExit("M9 diagnostics: handle_retrieve_cookie_state a change en amont")
    data = data.replace(ancien_cookie, nouveau_cookie, 1)

    request_server_cpp.write_text(data)


# RequestServer voit-il seulement arriver la requete ? Sans cette sonde, un
# arret avant le premier etat serait indiscernable d'un message IPC jamais
# delivre — deux causes qui n'ont pas le meme correctif.
connection_rs_cpp = root / "Services/RequestServer/ConnectionFromClient.cpp"
data = connection_rs_cpp.read_text()

if "M9_RS_START_RECU" not in data:
    if "<cstdlib>" not in data:
        premiere = data.index("#include ")
        data = data[:premiere] + "#include <cstdlib>\n" + data[premiere:]

    ancien = """    note_event_tick("ipc-start-request"sv);
    dbgln_if(REQUESTSERVER_DEBUG, "RequestServer: start_request({}, {})", request_id, url);"""
    nouveau = """    note_event_tick("ipc-start-request"sv);
    dbgln_if(REQUESTSERVER_DEBUG, "RequestServer: start_request({}, {})", request_id, url);
#if defined(BOUCHAUD_PORT)
    if (getenv("BOUCHAUD_M9") != nullptr)
        outln("[ladybird-bouchaud] M9_RS_START_RECU id={} methode={} url={}", request_id, method, url);
#endif"""
    if ancien not in data:
        raise SystemExit("M9 diagnostics: ConnectionFromClient::start_request a change en amont")
    data = data.replace(ancien, nouveau, 1)

    # Symetrique : la reponse du client au cookie. Si elle n'arrive pas, la
    # requete reste en `RetrieveCookie` pour toujours et rien ne le dit.
    ancien_recu = """void ConnectionFromClient::retrieved_http_cookie("""
    if ancien_recu in data:
        i = data.index(ancien_recu)
        j = data.index("{\n", i) + 2
        data = data[:j] + """#if defined(BOUCHAUD_PORT)
    if (getenv("BOUCHAUD_M9") != nullptr)
        outln("[ladybird-bouchaud] M9_RS_COOKIE_RECU id={}", request_id);
#endif
""" + data[j:]

    connection_rs_cpp.write_text(data)


# ---------------------------------------------------------------------------
# M13 DNS: instrument LibDNS between RequestServer::DNSLookup and the UDP wire.
# Temporary local diagnostics; remove before the final cleanup PR.
# ---------------------------------------------------------------------------
m13_dns_script = Path(__file__).with_name("prepare-m13-dns-diagnostics.py")
exec(compile(m13_dns_script.read_text(), str(m13_dns_script), "exec"))

# The next adaptation is deliberately kept in its own small source-patch file:
# it changes one upstream Fetch behaviour (Document-body pausing), while this
# file remains diagnostics/observability. browser-upstream.sh already invokes
# this script, so chain the local-navigation patch here without another build
# entry point.
navigation_script = Path(__file__).with_name("prepare-m9-navigation.py")
exec(compile(navigation_script.read_text(), str(navigation_script), "exec"))

print("Bouchaud M9 body/Fetch diagnostics applied to", root)

# ---------------------------------------------------------------------------
# M14 DNS: let timer-driven in-flight lookups reach LibDNS retransmission
# instead of joining the existing pending Promise before the UDP wire write.
# ---------------------------------------------------------------------------
m14_dns_script = Path(__file__).with_name("prepare-m14-dns-retry-fix.py")
exec(compile(m14_dns_script.read_text(), str(m14_dns_script), "exec"))
