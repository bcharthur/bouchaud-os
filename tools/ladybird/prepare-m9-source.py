#!/usr/bin/env python3
# Sondes et contournements reseau de LibRequests (worktree jetable).
#
# BOUCHAUD_UI_V1 : ce script portait aussi le mode M9, ou WebContent tournait
# sans processus navigateur (fd RequestServer herite, page amorcee par
# WebContent, pont de capture). Ce mode est retire : WebContent est lance par
# UI/Bouchaud comme sur Linux. Ne restent que les sondes du chemin retour
# RequestServer -> WebContent et le drainage du corps (reveil de notificateur
# manquant sur la boucle d'evenements Bouchaud), tous deux gardes par
# BOUCHAUD_M9, que le noyau exporte toujours et qui ne commande plus que cette
# instrumentation reseau.

from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-m9-source.py <ladybird-worktree>")

root = Path(sys.argv[1])


def replace_once(path: Path, old: str, new: str, label: str):
    data = path.read_text()
    if new in data:
        return
    if old not in data:
        raise SystemExit(f"M9 pattern not found ({label}) in {path}")
    path.write_text(data.replace(old, new, 1))


# --- Sondes du chemin retour RequestServer -> WebContent ----------------------
#
# Le run precedent a etabli que le GET part, que la fixture repond 200, et que
# la navigation n'est ni commitee ni annulee : elle se tait. Le trou est donc
# dans les messages **retour**.
#
# Ce chemin n'est pas un simple flux d'octets sur la socket IPC : RequestServer
# cree une paire de sockets (`RequestPipe`), en passe le bout lecteur a
# WebContent par SCM_RIGHTS, et y ecrit le corps. Deux messages jalonnent
# l'echange — `request_started`, qui porte le descripteur, puis
# `request_finished`. Savoir lequel arrive et lequel manque partage le probleme
# en deux moities dont une seule restera a explorer.
requests_cpp = root / "Libraries/LibRequests/RequestClient.cpp"
data = requests_cpp.read_text()

if "bouchaud_m9_trace" not in data:
    anchor = "namespace Requests {\n"
    if anchor not in data:
        raise SystemExit("M9 RequestClient: namespace Requests introuvable")
    helper = anchor + "\nstatic bool bouchaud_m9_trace()\n{\n    return getenv(\"BOUCHAUD_M9\") != nullptr;\n}\n"
    data = data.replace(anchor, helper, 1)
    # `getenv` et `outln`. Le fichier utilise deja `warnln`, donc AK/Format.h
    # arrive transitivement — mais une inclusion transitive est une dependance
    # que personne n'a ecrite et que rien ne garantit.
    if "<cstdlib>" not in data:
        first_include = data.index("#include ")
        data = data[:first_include] + "#include <cstdlib>\n#include <AK/Format.h>\n" + data[first_include:]

    sondes = [
        ("void RequestClient::request_started(u64 request_id, IPC::File response_file)\n{\n",
         "M9_RS_REQUEST_STARTED id={}", "request_id"),
        ("void RequestClient::request_finished(u64 request_id, u64 total_size, RequestTimingInfo timing_info, Optional<NetworkError> network_error)\n{\n",
         "M9_RS_REQUEST_FINISHED id={} taille={}", "request_id, total_size"),
        # Le message central du trio. Le corps peut traverser entierement sans
        # que LibWeb n'ait de reponse a commiter : c'est ici que le code de
        # statut et les en-tetes arrivent, et sans eux il n'y a pas de document.
        ("void RequestClient::headers_became_available(u64 request_id, Vector<HTTP::Header> response_headers, Optional<u32> status_code, Optional<String> reason_phrase, Optional<IPC::File> javascript_bytecode_file, u64 javascript_bytecode_size, Optional<u64> javascript_bytecode_cache_vary_key, CameFromCache came_from_cache)\n{\n",
         "M9_RS_HEADERS id={} statut={} nb={}", "request_id, status_code.value_or(0), response_headers.size()"),
    ]
    for signature, message, args in sondes:
        if signature not in data:
            raise SystemExit("M9 RequestClient: signature introuvable -> " + message)
        data = data.replace(
            signature,
            signature + "    if (bouchaud_m9_trace())\n        outln(\"[ladybird-bouchaud] " + message + "\", " + args + ");\n",
            1)

    requests_cpp.write_text(data)


# Bouchaud M9 receives the response through a RequestPipe socketpair. The
# RequestServer marker above proves that the complete body has already been
# written to that pipe before `request_finished` reaches WebContent. On the
# current Bouchaud event loop the read notifier can nevertheless remain asleep,
# leaving Fetch waiting forever with the payload already queued. Drain the
# ready pipe once when completion is announced. This keeps the normal LibWeb
# streaming path and only compensates for the missing notifier wake-up on the
# Bouchaud M9 port; other platforms and M8 stay byte-for-byte untouched.
request_cpp = root / "Libraries/LibRequests/Request.cpp"
data = request_cpp.read_text()

if "M9_BODY_DRAIN_BEGIN" not in data:
    old = '''void Request::did_finish(Badge<RequestClient>, u64 total_size, RequestTimingInfo const& timing_info, Optional<NetworkError> const& network_error)
{
    auto effective_network_error = m_body_delivery_error.has_value() ? m_body_delivery_error : network_error;
    if (on_finish)
        on_finish(total_size, timing_info, effective_network_error);
}'''
    new = '''void Request::did_finish(Badge<RequestClient>, u64 total_size, RequestTimingInfo const& timing_info, Optional<NetworkError> const& network_error)
{
    auto effective_network_error = m_body_delivery_error.has_value() ? m_body_delivery_error : network_error;
#if defined(BOUCHAUD_PORT)
    // Une reponse file-backed/cache-backed peut avoir un payload valide sans
    // ReadStream. Le callback du notifier dereference read_stream : ne jamais
    // le forcer dans ce cas, laisser la logique upstream appeler on_finish.
    if (getenv("BOUCHAUD_M9") != nullptr && m_internal_stream_data && !m_internal_stream_data->read_stream)
        outln("[ladybird-bouchaud] M9_BODY_DRAIN_SKIP reason=no-read-stream total={}", total_size);

    if (getenv("BOUCHAUD_M9") != nullptr && m_internal_stream_data && m_internal_stream_data->read_stream && m_internal_stream_data->read_notifier && m_internal_stream_data->read_notifier->on_activation) {
        outln("[ladybird-bouchaud] M9_BODY_DRAIN_BEGIN total={}", total_size);
        m_internal_stream_data->read_notifier->on_activation();
        outln("[ladybird-bouchaud] M9_BODY_DRAIN_DONE total={}", total_size);
    }
#endif
    if (on_finish)
        on_finish(total_size, timing_info, effective_network_error);
}'''
    if old not in data:
        raise SystemExit("M9 Request body completion hook changed upstream")
    data = data.replace(old, new, 1)
    if "<cstdlib>" not in data:
        first_include = data.index("#include ")
        data = data[:first_include] + "#include <cstdlib>\n#include <AK/Format.h>\n" + data[first_include:]
    request_cpp.write_text(data)


print("Bouchaud LibRequests network probes applied to", root)