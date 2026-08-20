#!/usr/bin/env python3
# M16 — instrumenter LibDNS, et seulement LibDNS.
#
# ## Ce qu'on cherche
#
# Une navigation vers `https://example.com/` atteint desormais RequestServer et
# s'arrete net :
#
#     M9_RS_STATE     id=0 Init -> DNSLookup
#     M9_RS_DNS_QUERY id=0 host=example.com serveur=10.0.2.3:53
#     (plus rien pendant cinq minutes)
#
# La sonde `M9_RS_DNS_QUERY` est posee *juste avant* `dns.lookup()`. Tout ce qui
# suit se passe dans `LibDNS/Resolver.h`, ou rien ne parle : les traces internes
# sont derriere `DNS_DEBUG` et `REQUESTSERVER_WIRE_DEBUG`, deux drapeaux de
# compilation qui recompileraient tout RequestServer en mode verbeux et
# noieraient la console serie.
#
# ## Pourquoi ces sondes-la
#
# Le silence observe a **trois** explications incompatibles, et aucune mesure ne
# les separe aujourd'hui :
#
#  1. `has_connection()` echoue — la socket UDP vers 10.0.2.3:53 n'a pas pu etre
#     creee. LibDNS bascule alors, sans le dire, sur le **resolveur systeme**
#     (`getaddrinfo` dans un `ThreadPool`). Sur un binaire statique sans NSS ni
#     `/etc/resolv.conf`, ce chemin peut ne jamais rendre la main — et il n'a ni
#     delai ni retransmission, ce qui expliquerait exactement cinq minutes de
#     rien.
#  2. La requete part mais la reponse ne revient pas. Le minuteur de repetition
#     devrait alors tirer cinq fois a une seconde d'intervalle puis rejeter la
#     promesse. S'il ne tire pas, c'est le minuteur qu'il faut regarder.
#  3. La reponse revient mais ne s'analyse pas — un datagramme tronque, un
#     identifiant qui ne correspond a aucune recherche en attente.
#
# Chaque sonde ci-dessous est choisie pour **eliminer** une de ces branches, pas
# pour decrire ce qui va bien. `M16_DNS_LOOKUP path=` suffit a lui seul a
# trancher entre (1) et les autres : `lookup_path` est une variable qu'upstream
# tient deja a jour dans chaque branche, on ne fait que la rendre visible.
#
# Les sondes sont **temporaires** et bornees a `LibDNS` : aucune ligne
# supplementaire dans le noyau, la pile UDP ou le rendu.

from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-m16-dns.py <ladybird-worktree>")

root = Path(sys.argv[1])
resolver = root / "Libraries/LibDNS/Resolver.h"
data = resolver.read_text()

if "M16_DNS_LOOKUP" in data:
    print("M16 DNS probes already applied")
    raise SystemExit(0)

# `getenv` n'est pas garanti par les inclusions de cet en-tete.
if "<cstdlib>" not in data:
    premiere = data.index("#include ")
    data = data[:premiere] + "#include <cstdlib>\n" + data[premiere:]

# Un seul predicat, pour que toutes les sondes s'allument ensemble et se
# taisent ensemble. `BOUCHAUD_M9` est deja la variable que le reste du portage
# utilise : en introduire une seconde ferait deux etats a se rappeler.
anchor = "namespace DNS {\n"
if anchor not in data:
    raise SystemExit("M16: namespace DNS introuvable")
data = data.replace(
    anchor,
    anchor + """
#if defined(BOUCHAUD_PORT)
// Sondes M16 : voir tools/ladybird/prepare-m16-dns.py.
static inline bool bouchaud_m16_trace()
{
    return getenv("BOUCHAUD_M9") != nullptr;
}
#endif
""",
    1,
)

sondes = []

# --- 1. Quel chemin la recherche a-t-elle pris ? -----------------------------
#
# La reponse a elle seule elimine ou confirme la bascule silencieuse vers le
# resolveur systeme. Upstream ne journalise `lookup_path` que sous
# `REQUESTSERVER_WIRE_DEBUG` **et** seulement si la partie synchrone a dure plus
# de cinq millisecondes — c'est-a-dire jamais dans le cas qui nous occupe, ou
# elle rend la main immediatement.
sondes.append((
    "chemin de recherche",
    """        ScopeGuard log_guard = [&] {
            if constexpr (!REQUESTSERVER_WIRE_DEBUG)
                return;""",
    """        ScopeGuard log_guard = [&] {
#if defined(BOUCHAUD_PORT)
            if (bouchaud_m16_trace())
                outln("[ladybird-bouchaud] M16_DNS_LOOKUP name={} path={}", name, lookup_path);
#endif
            if constexpr (!REQUESTSERVER_WIRE_DEBUG)
                return;""",
))

# --- 2. La socket vers le resolveur a-t-elle pu etre creee ? -----------------
#
# `m_create_socket()` echoue en silence : l'erreur part dans un
# `dbgln_if(DNS_DEBUG, ...)`, donc nulle part. C'est precisement l'entree de la
# branche « resolveur systeme ».
sondes.append((
    "creation de socket",
    """            auto create_result = m_create_socket();
            if (create_result.is_error()) {
                dbgln_if(DNS_DEBUG, "DNS: Failed to create socket: {}", create_result.error());
                return false;
            }""",
    """            auto create_result = m_create_socket();
            if (create_result.is_error()) {
#if defined(BOUCHAUD_PORT)
                if (bouchaud_m16_trace())
                    outln("[ladybird-bouchaud] M16_DNS_SOCKET_ECHEC erreur={}", create_result.error());
#endif
                dbgln_if(DNS_DEBUG, "DNS: Failed to create socket: {}", create_result.error());
                return false;
            }
#if defined(BOUCHAUD_PORT)
            if (bouchaud_m16_trace())
                outln("[ladybird-bouchaud] M16_DNS_SOCKET_OK");
#endif""",
))

# --- 3. La requete est-elle reellement partie, et de combien d'octets ? ------
#
# `write_until_depleted` sur une socket UDP connectee est le `sendto` reel. Zero
# octet ecrit sans erreur, ou une erreur, sont deux pannes differentes.
sondes.append((
    "emission de la requete",
    """        auto write_result = m_socket.with_write_locked([&](auto& socket) {
            return (*socket)->write_until_depleted(query_bytes.bytes());
        });
        if (write_result.is_error()) {""",
    """        auto write_result = m_socket.with_write_locked([&](auto& socket) {
            return (*socket)->write_until_depleted(query_bytes.bytes());
        });
#if defined(BOUCHAUD_PORT)
        if (bouchaud_m16_trace()) {
            if (write_result.is_error())
                outln("[ladybird-bouchaud] M16_DNS_TX_ECHEC id={} erreur={}", query.header.id, write_result.error());
            else
                outln("[ladybird-bouchaud] M16_DNS_TX id={} octets={}", query.header.id, query_bytes.size());
        }
#endif
        if (write_result.is_error()) {""",
))

# --- 4. Le minuteur de retransmission tire-t-il ? ----------------------------
#
# Cinq tirs a une seconde d'intervalle, puis rejet. Leur absence dit que la
# boucle d'evenements ne fait pas avancer ses minuteurs — une panne qui n'a rien
# a voir avec le reseau.
sondes.append((
    "minuteur de retransmission",
    """                  p->repeat_timer->on_timeout = [=, this] {
                      (void)lookup(name, class_, desired_types, { .validate_dnssec_locally = options.validate_dnssec_locally, .repeating_lookup = p });
                  };""",
    """                  p->repeat_timer->on_timeout = [=, this] {
#if defined(BOUCHAUD_PORT)
                      if (bouchaud_m16_trace())
                          outln("[ladybird-bouchaud] M16_DNS_REPEAT id={} tentative={}", p->id, p->times_repeated + 1);
#endif
                      (void)lookup(name, class_, desired_types, { .validate_dnssec_locally = options.validate_dnssec_locally, .repeating_lookup = p });
                  };""",
))

# --- 5. La socket se declare-t-elle lisible ? --------------------------------
#
# `on_ready_to_read` est arme par un `Core::Notifier`, donc par le `poll()` de la
# boucle d'evenements sur le descripteur UDP. S'il ne se declenche jamais alors
# qu'un datagramme est arrive, c'est la notification qu'il faut regarder, pas la
# pile reseau.
sondes.append((
    "reveil en lecture",
    """    void process_incoming_messages()
    {
        while (true) {""",
    """    void process_incoming_messages()
    {
#if defined(BOUCHAUD_PORT)
        if (bouchaud_m16_trace())
            outln("[ladybird-bouchaud] M16_DNS_READY");
#endif
        while (true) {""",
))

# --- 6. Le message recu s'analyse-t-il, et correspond-il a une attente ? -----
sondes.append((
    "analyse du message",
    """            auto message = message_or_err.release_value();
            auto result = m_pending_lookups.with_write_locked([&](auto& lookups) -> ErrorOr<void> {
                auto* lookup = lookups->find(message.header.id);
                if (!lookup)
                    return Error::from_string_literal("No pending lookup found for this message");""",
    """            auto message = message_or_err.release_value();
#if defined(BOUCHAUD_PORT)
            if (bouchaud_m16_trace())
                outln("[ladybird-bouchaud] M16_DNS_RX id={} rcode={} reponses={}",
                    message.header.id,
                    static_cast<u16>(message.header.options.response_code()),
                    message.answers.size());
#endif
            auto result = m_pending_lookups.with_write_locked([&](auto& lookups) -> ErrorOr<void> {
                auto* lookup = lookups->find(message.header.id);
                if (!lookup)
                    return Error::from_string_literal("No pending lookup found for this message");""",
))

for label, ancien, nouveau in sondes:
    if ancien not in data:
        raise SystemExit(f"M16: ancre introuvable ({label}) — LibDNS/Resolver.h a change en amont")
    data = data.replace(ancien, nouveau, 1)

resolver.write_text(data)
print(f"M16 DNS probes applied to {resolver}")
