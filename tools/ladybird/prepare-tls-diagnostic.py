#!/usr/bin/env python3
"""Faire dire a curl *pourquoi* la poignee de main TLS a echoue.

## Ce qu'on observe

    https://www.wikipedia.org/   TLS   OK
    https://www.google.com/      TLS   « SSL handshake failed »

Meme machine, meme pile, meme magasin d'autorites, DNS resolu dans les deux cas
(huit adresses IPv4 pour Google), connexion etablie, `Fetch` demarre. Puis rien.

## Ce que ce message dit deja, et ce qu'il ne dit pas

Il dit une chose utile : `SSLHandshakeFailed` vient de `CURLE_SSL_CONNECT_ERROR`
et **non** de `CURLE_PEER_FAILED_VERIFICATION`, qui a son propre libelle
(`SSLVerificationFailed`). Ce n'est donc pas un probleme de confiance dans le
certificat — c'est la negociation elle-meme qui casse.

Il ne dit rien de plus, et pour une raison precise : Ladybird ne pose nulle part
`CURLOPT_ERRORBUFFER`. Sans ce tampon, curl ne peut rendre que le libelle
generique de son code d'erreur. Avec, il rend la phrase d'OpenSSL — par exemple

    error:0A000410:SSL routines::sslv3 alert handshake failure
    error:0A000126:SSL routines::unexpected eof while reading
    OpenSSL SSL_connect: Connection reset by peer in connection to www.google.com

et ces trois phrases designent trois pannes differentes : un parametre refuse
par le serveur, une connexion coupee en cours de poignee de main, un rejet
immediat. Aucun raisonnement ne permet de choisir entre elles ; la phrase, si.

## Ce que ce script ajoute

1. `CURLOPT_ERRORBUFFER` sur chaque transfert, et la phrase imprimee quand le
   transfert echoue. C'est gratuit : un tampon de 256 octets par requete, aucun
   travail supplementaire tant que rien n'echoue.

2. Le contexte de la connexion au moment de l'echec — adresse et port
   reellement joints, version TLS negociee si elle l'a ete, resultat de
   verification du certificat. Cela separe « je n'ai jamais parle a ce
   serveur » de « j'ai parle et il a raccroche ».

3. Sur demande (`BOUCHAUD_CURL_TRACE=1`), la trace verbeuse de curl : chaque
   etape de la poignee de main, dans l'ordre — ClientHello, ServerHello, chaine
   de certificats, alerte. Les corps de reponse ne sont jamais traces ; seules
   les lignes de texte et les en-tetes le sont, sinon la console serie serait
   noyee par le HTML.

Rien de tout cela ne relache la verification TLS, ne fixe une version de
protocole, ne code en dur une adresse ni ne modifie ce qui est envoye. Le
handshake reste exactement celui d'upstream : on le regarde, on ne le change
pas.
"""
from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-tls-diagnostic.py <ladybird-worktree>")

root = Path(sys.argv[1])


def substitute(path: Path, old: str, new: str, label: str) -> None:
    data = path.read_text()
    if new in data:
        return
    if old not in data:
        raise SystemExit(f"TLS : ancre introuvable ({label}) dans {path}")
    path.write_text(data.replace(old, new, 1))


# ---------------------------------------------------------------------------
# 1. Le tampon d'erreur, membre de la requete.
# ---------------------------------------------------------------------------

request_h = root / "Services/RequestServer/Request.h"
substitute(
    request_h,
    "    Optional<int> m_curl_result_code;",
    """    Optional<int> m_curl_result_code;

#if defined(BOUCHAUD_PORT)
    // Voir tools/ladybird/prepare-tls-diagnostic.py. curl y ecrit la phrase
    // d'erreur d'OpenSSL ; sans ce tampon il ne rend que le libelle generique
    // de son code, qui ne distingue pas trois pannes distinctes.
    //
    // 256 et non `CURL_ERROR_SIZE` : cet en-tete evite deliberement d'inclure
    // curl (le manche est un `void*`, `curl_slist` n'est que declare), et une
    // inclusion ici la propagerait a tous ses consommateurs. Un
    // `static_assert` dans Request.cpp verifie que la taille suffit vraiment.
    char m_bouchaud_curl_error[256] {};
#endif""",
    "tampon d'erreur",
)

# ---------------------------------------------------------------------------
# 2. Le poser sur le transfert, et brancher la trace optionnelle.
# ---------------------------------------------------------------------------

request_cpp = root / "Services/RequestServer/Request.cpp"

substitute(
    request_cpp,
    """    set_option(CURLOPT_CUSTOMREQUEST, m_method.characters());
    set_option(CURLOPT_FOLLOWLOCATION, 0);""",
    """#if defined(BOUCHAUD_PORT)
    m_bouchaud_curl_error[0] = '\\0';
    set_option(CURLOPT_ERRORBUFFER, m_bouchaud_curl_error);
    if (bouchaud_curl_trace()) {
        set_option(CURLOPT_VERBOSE, 1L);
        set_option(CURLOPT_DEBUGFUNCTION, bouchaud_curl_debug);
    }
#endif
    set_option(CURLOPT_CUSTOMREQUEST, m_method.characters());
    set_option(CURLOPT_FOLLOWLOCATION, 0);""",
    "pose du tampon",
)

# Le rappel de trace et son predicat, poses juste avant `handle_fetch_state`.
substitute(
    request_cpp,
    "void Request::handle_fetch_state()\n{",
    """#if defined(BOUCHAUD_PORT)
// Voir tools/ladybird/prepare-tls-diagnostic.py.
static bool bouchaud_curl_trace()
{
    static bool const value = getenv("BOUCHAUD_CURL_TRACE") != nullptr;
    return value;
}

// Les corps de reponse ne sont jamais traces : une page d'accueil ferait defiler
// cent kibioctets de HTML sur la console serie et rendrait la trace illisible.
// Seules les lignes de texte de curl — c'est-a-dire la poignee de main — et les
// en-tetes sont reprises.
static int bouchaud_curl_debug(CURL*, curl_infotype type, char* data, size_t size, void*)
{
    char const* etiquette = nullptr;
    switch (type) {
    case CURLINFO_TEXT:
        etiquette = "*";
        break;
    case CURLINFO_HEADER_OUT:
        etiquette = ">";
        break;
    case CURLINFO_HEADER_IN:
        etiquette = "<";
        break;
    default:
        return 0;
    }

    // curl termine ses lignes par un saut ; `outln` en ajoute un autre.
    while (size > 0 && (data[size - 1] == '\\n' || data[size - 1] == '\\r'))
        --size;
    if (size == 0)
        return 0;

    outln("[ladybird-bouchaud] CURL {} {}", etiquette, StringView { data, size });
    return 0;
}
#endif

void Request::handle_fetch_state()
{""",
    "rappel de trace",
)

# ---------------------------------------------------------------------------
# 3. Imprimer la raison quand le transfert echoue.
# ---------------------------------------------------------------------------

substitute(
    request_cpp,
    """        if (m_curl_result_code != CURLE_OK) {
            m_network_error = curl_code_to_network_error(*m_curl_result_code);
""",
    """        if (m_curl_result_code != CURLE_OK) {
            m_network_error = curl_code_to_network_error(*m_curl_result_code);

#if defined(BOUCHAUD_PORT)
            // La phrase d'OpenSSL, puis le contexte de la connexion. Ensemble
            // ils separent « je n'ai jamais parle a ce serveur » de « j'ai
            // parle et la negociation a echoue ».
            char* adresse = nullptr;
            long port = 0;
            long verification = 0;
            long version_http = 0;
            curl_easy_getinfo(m_curl_easy_handle, CURLINFO_PRIMARY_IP, &adresse);
            curl_easy_getinfo(m_curl_easy_handle, CURLINFO_PRIMARY_PORT, &port);
            curl_easy_getinfo(m_curl_easy_handle, CURLINFO_SSL_VERIFYRESULT, &verification);
            curl_easy_getinfo(m_curl_easy_handle, CURLINFO_HTTP_VERSION, &version_http);
            outln("[ladybird-bouchaud] RS_ECHEC url={} code={} ({}) pair={}:{} verif_cert={} http={}",
                m_url.to_byte_string(),
                *m_curl_result_code,
                curl_easy_strerror(static_cast<CURLcode>(*m_curl_result_code)),
                adresse ? adresse : "(aucune)",
                port,
                verification,
                version_http);
            if (m_bouchaud_curl_error[0] != '\\0')
                outln("[ladybird-bouchaud] RS_ECHEC_DETAIL {}", m_bouchaud_curl_error);
            else
                outln("[ladybird-bouchaud] RS_ECHEC_DETAIL (curl n'a rien ecrit)");
#endif
""",
    "impression de la raison",
)

data = request_cpp.read_text()
if "#include <cstdlib>" not in data:
    premiere = data.index("#include ")
    request_cpp.write_text(data[:premiere] + "#include <cstdlib>\n" + data[premiere:])

print("Diagnostic TLS applique a", root)
