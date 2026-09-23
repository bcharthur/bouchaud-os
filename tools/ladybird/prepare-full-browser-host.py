#!/usr/bin/env python3
'''
Bouchaud Browser Host complet - consolidation des patches v2 + NEXT.

S'applique au worktree Ladybird jetable apres les adaptations existantes.

Cible:
    Bouchaud WM
      -> BouchaudBrowserHost (WebView::Application upstream)
          -> RequestServer
          -> ImageDecoder
          -> Compositor (--force-cpu-painting)
          -> WebContent
          -> WebWorker(s) a la demande via WorkerProcessManager

Le chrome M11 reste un pont GUI temporaire. Site isolation reste donc
volontairement desactivee pendant cette phase.
'''

from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-full-browser-host.py <ladybird-worktree>")

root = Path(sys.argv[1])


def replace_once(path: Path, old: str, new: str, label: str) -> None:
    data = path.read_text()
    if new in data:
        return
    if old not in data:
        raise SystemExit(f"BrowserHost: ancre introuvable ({label}) dans {path}")
    path.write_text(data.replace(old, new, 1))


def ensure_include(path: Path, include: str, anchor: str, label: str) -> None:
    data = path.read_text()
    if include in data:
        return
    if anchor not in data:
        raise SystemExit(f"BrowserHost: ancre include introuvable ({label}) dans {path}")
    path.write_text(data.replace(anchor, include + "\n" + anchor, 1))


# 1. Ajouter un hote base sur WebView::Application upstream.
services_cmake = root / "Services/CMakeLists.txt"
data = services_cmake.read_text()
block = '''if (BOUCHAUD_PORT)
    add_subdirectory(BouchaudBrowserHost)
endif()
'''
if block not in data:
    anchor = "add_subdirectory(WebWorker)\n"
    if anchor not in data:
        raise SystemExit("BrowserHost: WebWorker absent de Services/CMakeLists.txt")
    services_cmake.write_text(data.replace(anchor, anchor + "\n" + block, 1))

host_dir = root / "Services/BouchaudBrowserHost"
host_dir.mkdir(parents=True, exist_ok=True)

(host_dir / "CMakeLists.txt").write_text(r'''add_executable(BouchaudBrowserHost main.cpp)

target_include_directories(BouchaudBrowserHost PRIVATE
    ${LADYBIRD_SOURCE_DIR}
    ${LADYBIRD_SOURCE_DIR}/Services
)

target_link_libraries(BouchaudBrowserHost PRIVATE
    AK
    LibCore
    LibFileSystem
    LibGfx
    LibImageDecoderClient
    LibIPC
    LibJS
    LibMain
    LibRequests
    LibSync
    LibURL
    LibWakeLock
    LibWeb
    LibWebView
    OpenSSL::Crypto
    OpenSSL::SSL
)

if (BOUCHAUD_PORT)
    target_link_options(BouchaudBrowserHost PRIVATE
        -static-pie
        LINKER:--allow-multiple-definition
    )
    set_target_properties(BouchaudBrowserHost PROPERTIES
        SKIP_BUILD_RPATH TRUE
        BUILD_WITH_INSTALL_RPATH FALSE
        INSTALL_RPATH ""
    )
endif()
''')

(host_dir / "main.cpp").write_text(r'''/*
 * Bouchaud Browser Host.
 *
 * Ce binaire n'est pas un nouveau moteur. Il instancie WebView::Application
 * upstream afin de retrouver l'architecture Browser -> helper processes.
 */

#include <AK/ByteString.h>
#include <AK/StringView.h>
#include <AK/Vector.h>
#include <LibMain/Main.h>
#include <LibWebView/Application.h>

#include <cstdlib>
#include <sys/stat.h>
#include <unistd.h>

namespace {

static int env_dimension(char const* name, int fallback)
{
    auto* value = getenv(name);
    if (!value || !*value)
        return fallback;
    auto parsed = atoi(value);
    return parsed > 0 ? parsed : fallback;
}

class BouchaudBrowserApplication final : public WebView::Application {
public:
    BouchaudBrowserApplication()
        : WebView::Application(ByteString { "/usr/libexec/ladybird" })
    {
    }

    ErrorOr<int> run(Main::Arguments const& arguments)
    {
        TRY(initialize(arguments));
        // `BROWSER_HOST_START` est imprime AVANT `run()` : il ne prouve que le
        // demarrage du processus. Cette ligne-ci prouve que `initialize()` est
        // alle au bout -- chemins de ressources, magasins SQL, services --
        // juste avant d'entrer dans la boucle d'evenements.
        outln("[ladybird-bouchaud] BROWSER_HOST_INITIALIZED");
        return execute();
    }

    virtual Optional<String> system_font_family() const override
    {
        // BOUCHAUD_POLICE_SYSTEME_V1
        //
        // C'etait "SerenitySans" : la seule police de texte de l'arbre
        // Ladybird epingle, et une police d'INTERFACE geometrique, sans la
        // plupart des lettres accentuees.
        //
        // Ladybird s'en sert pour la police par defaut, pour `system-ui`, et
        // comme dernier recours quand aucune famille demandee n'est trouvee.
        // C'est-a-dire, en pratique, pour presque tout : la page d'accueil de
        // Google demande `arial, sans-serif` et ne telecharge aucune police.
        //
        // D'ou ce qu'on voyait : tout le Web en lettres geometriques, et un
        // carre vide a la place de chaque lettre accentuee --
        // « Avant d'acc[carre]der [carre] Google ».
        //
        // DejaVu Sans est deposee par le noyau dans /usr/share/fonts
        // (`kernel::sysroot::install_fonts`) et indexee par le fontconfig du
        // depot. Elle couvre le Latin accentue, et c'est une police de TEXTE.
        return "DejaVu Sans"_string;
    }

protected:
    virtual bool should_coordinate_browser_process() const override
    {
        return false;
    }
};

static Main::Arguments make_arguments(
    Vector<ByteString>& storage,
    Vector<char*>& argv,
    Vector<StringView>& strings)
{
    argv.ensure_capacity(storage.size());
    strings.ensure_capacity(storage.size());

    for (auto& argument : storage) {
        argv.append(const_cast<char*>(argument.characters()));
        strings.append(argument.view());
    }

    return Main::Arguments {
        .argc = static_cast<int>(argv.size()),
        .argv = argv.data(),
        .strings = strings.span(),
    };
}

}

ErrorOr<int> ladybird_main(Main::Arguments)
{
    setenv("BOUCHAUD_BROWSER_HOST", "1", 1);

    mkdir("/tmp", 0777);
    mkdir("/tmp/ladybird", 0777);
    mkdir("/tmp/ladybird-profile", 0777);
    mkdir("/tmp/ladybird-runtime", 0700);
    mkdir("/tmp/ladybird-data", 0777);
    mkdir("/tmp/ladybird-cache", 0777);
    mkdir("/tmp/fontconfig", 0777);

    // Core::StandardPaths::runtime_directory() retombe sinon sur
    // /run/user/<uid> via le chemin Linux, qui n'existe pas dans Bouchaud OS.
    chmod("/tmp/ladybird-runtime", 0700);

    setenv("HOME", "/tmp/ladybird", 1);
    setenv("XDG_CONFIG_HOME", "/tmp/ladybird", 1);
    setenv("XDG_RUNTIME_DIR", "/tmp/ladybird-runtime", 1);
    setenv("XDG_DATA_HOME", "/tmp/ladybird-data", 1);
    setenv("XDG_CACHE_HOME", "/tmp/ladybird-cache", 1);

    constexpr char fontconfig_file[] = "/usr/share/ladybird/fontconfig/fonts.conf";
    if (access(fontconfig_file, R_OK) == 0) {
        setenv("FONTCONFIG_FILE", fontconfig_file, 1);
        setenv("FONTCONFIG_PATH", "/usr/share/ladybird/fontconfig", 1);
    }

    auto width = env_dimension("BO_SURFACE_WIDTH", 1100);
    auto full_height = env_dimension("BO_SURFACE_HEIGHT", 604);
    auto height = full_height;
    if (getenv("BOUCHAUD_M11") && height > 36)
        height -= 36;

    char const* url = getenv("BOUCHAUD_M9_URL");
    if (!url || !*url)
        url = "https://example.com/";

    char const* dns = getenv("BOUCHAUD_DNS_SERVER");
    if (!dns || !*dns)
        dns = "10.0.2.3";

    Vector<ByteString> arguments;
    arguments.append("BouchaudBrowserHost");
    arguments.append("--headless=manual");
    arguments.append("--window-width");
    arguments.append(ByteString::number(width));
    arguments.append("--window-height");
    arguments.append(ByteString::number(height));
    arguments.append("--force-cpu-painting");
    arguments.append("--force-fontconfig");
    arguments.append("--disable-sandbox");
    arguments.append("--disable-http-disk-cache");
    // Premiere validation multiprocessus: garder les vraies classes Ladybird
    // CookieJar/StorageJar/HSTSStore, mais sans persistance SQL.
    arguments.append("--disable-sql-database");
    arguments.append("--disable-async-scrolling");
    arguments.append("--site-isolation=disable");
    arguments.append("--profile-path");
    arguments.append("/tmp/ladybird-profile");
    arguments.append("--dns-server");
    arguments.append(dns);
    arguments.append("--dns-port");
    arguments.append("53");

    constexpr char ca_bundle[] = "/etc/ssl/certs/ca-certificates.crt";
    if (access(ca_bundle, R_OK) == 0) {
        arguments.append("--certificate");
        arguments.append(ca_bundle);
    }

    arguments.append(url);

    Vector<char*> argv;
    Vector<StringView> strings;
    auto host_arguments = make_arguments(arguments, argv, strings);

    outln("[ladybird-bouchaud] BROWSER_HOST_START architecture=WebView::Application");
    outln("[ladybird-bouchaud] BROWSER_HOST_VIEWPORT {}x{}", width, height);
    outln("[ladybird-bouchaud] BROWSER_HOST_URL {}", url);
    outln("[ladybird-bouchaud] BROWSER_HOST_PATHS runtime=/tmp/ladybird-runtime data=/tmp/ladybird-data cache=/tmp/ladybird-cache resources=/usr/share/ladybird");
    outln("[ladybird-bouchaud] BROWSER_HOST_STORAGE sql=disabled upstream-jars=in-memory");
    outln("[ladybird-bouchaud] BROWSER_HOST_SERVICES RequestServer ImageDecoder Compositor WebContent WebWorker:on-demand");

    // Trois etapes distinctes, et chacune se dit. Au run 32427953935 le
    // processus s'est tu apres `window.close()` sans que rien ne permette de
    // savoir OU : la boucle d'evenements ne quittait-elle pas, ou quittait-elle
    // pour se bloquer ensuite dans l'arret des services ? Un processus qui
    // disparait en silence oblige a deviner.
    int code = 0;
    {
        BouchaudBrowserApplication application;
        auto resultat = application.run(host_arguments);
        if (resultat.is_error()) {
            outln("[ladybird-bouchaud] BROWSER_HOST_EXIT erreur");
            warnln("BouchaudBrowserHost: {}", resultat.error());
            return resultat.release_error();
        }
        code = resultat.value();
        outln("[ladybird-bouchaud] BROWSER_HOST_EXIT boucle_quittee code={}", code);
    }
    outln("[ladybird-bouchaud] BROWSER_HOST_ARRET services fermes");
    return code;
}
''')


# 2. Core::Process: garder le vrai fork/exec upstream, mais Bouchaud ne fournit
# pas encore PR_SET_PDEATHSIG.
process_cpp = root / "Libraries/LibCore/Process.cpp"
if "BOUCHAUD_PORT: pas encore de PR_SET_PDEATHSIG" not in process_cpp.read_text():
    replace_once(
        process_cpp,
        '''    auto parent_pid = getpid();
    auto pid = fork();''',
        '''#if !defined(BOUCHAUD_PORT)
    auto parent_pid = getpid();
#endif
    auto pid = fork();''',
        "parent pid prctl",
    )
    replace_once(
        process_cpp,
        '''        if (prctl(PR_SET_PDEATHSIG, SIGKILL) < 0)
            report_errno_and_exit(errno);
        if (getppid() != parent_pid)
            _exit(127);''',
        '''#if defined(BOUCHAUD_PORT)
        // BOUCHAUD_PORT: pas encore de PR_SET_PDEATHSIG.
#else
        if (prctl(PR_SET_PDEATHSIG, SIGKILL) < 0)
            report_errno_and_exit(errno);
        if (getppid() != parent_pid)
            _exit(127);
#endif''',
        "prctl parent death",
    )


# 2b. M11 interactif partage le fd GUI et la surface du BrowserHost.
#
# Upstream Ladybird lance volontairement un WebContent "spare" apres la
# creation du WebContent actif. Ce spare est une optimisation normale, mais un
# descripteur GUI herite n'est pas une file multidestinataire : deux lecteurs
# se repartissent les messages clavier/souris et deux BouchaudChrome independants
# ecrivent ensuite dans la meme surface.
#
# Tant que M11 vit dans WebContent, une fenetre Bouchaud == un WebContent actif.
application_cpp = root / "Libraries/LibWebView/Application.cpp"
data = application_cpp.read_text()
spare_signature = "void Application::launch_spare_web_content_process()\n{\n"
spare_guard = "void Application::launch_spare_web_content_process()\n{\n#if defined(BOUCHAUD_PORT)\n    if (Core::Environment::has(\"BOUCHAUD_BROWSER_HOST\"sv) && Core::Environment::has(\"BOUCHAUD_M11\"sv)) {\n        static bool reported_m11_spare_disabled = false;\n        if (!reported_m11_spare_disabled) {\n            reported_m11_spare_disabled = true;\n            outln(\"[ladybird-bouchaud] BROWSER_HOST_M11_SPARE_DISABLED reason=shared-gui-stream\");\n        }\n        return;\n    }\n#endif\n"
if spare_guard not in data:
    if spare_signature not in data:
        raise SystemExit("BrowserHost: Application::launch_spare_web_content_process introuvable")
    data = data.replace(spare_signature, spare_guard, 1)
    application_cpp.write_text(data)


# 3. Le probe debugger Linux utilise /proc, absent de Bouchaud.
utilities_cpp = root / "Libraries/LibWebView/Utilities.cpp"
if "BOUCHAUD_PORT: pas de /proc/self/status" not in utilities_cpp.read_text():
    replace_once(
        utilities_cpp,
        '''ErrorOr<void> handle_attached_debugger()
{
#if defined(AK_OS_LINUX)''',
        '''ErrorOr<void> handle_attached_debugger()
{
#if defined(BOUCHAUD_PORT)
    // BOUCHAUD_PORT: pas de /proc/self/status.
    return {};
#elif defined(AK_OS_LINUX)''',
        "debugger /proc",
    )


# 3b. Resource root de plateforme.
#
# Bouchaud package les ressources dans /usr/share/ladybird. Le calcul generique
# de LibWebView ajoute normalement share/Lagom a un prefixe deduit du binaire.
# On conserve l'algorithme upstream ailleurs et on fixe seulement BOUCHAUD_PORT.
utilities_data = utilities_cpp.read_text()
if "BOUCHAUD_PORT_RESOURCE_ROOT" not in utilities_data:
    resource_old_begin = (
        '    s_ladybird_binary_path = move(ladybird_binary_path);\n\n'
        '    s_ladybird_resource_root = [] {'
    )
    resource_new_begin = (
        '    s_ladybird_binary_path = move(ladybird_binary_path);\n\n'
        '#if defined(BOUCHAUD_PORT)\n'
        '    // BOUCHAUD_PORT_RESOURCE_ROOT\n'
        '    s_ladybird_resource_root = ByteString { "/usr/share/ladybird" };\n'
        '#else\n'
        '    s_ladybird_resource_root = [] {'
    )
    resource_old_end = (
        '    }();\n\n'
        '    Core::ResourceImplementation::install(make<Core::ResourceImplementationFile>'
        '(MUST(String::from_byte_string(s_ladybird_resource_root))));'
    )
    resource_new_end = (
        '    }();\n'
        '#endif\n\n'
        '    Core::ResourceImplementation::install(make<Core::ResourceImplementationFile>'
        '(MUST(String::from_byte_string(s_ladybird_resource_root))));'
    )

    if resource_old_begin not in utilities_data:
        raise SystemExit("BrowserHost: debut platform_init/resource root introuvable")
    utilities_data = utilities_data.replace(resource_old_begin, resource_new_begin, 1)

    if resource_old_end not in utilities_data:
        raise SystemExit("BrowserHost: fin platform_init/resource root introuvable")
    utilities_data = utilities_data.replace(resource_old_end, resource_new_end, 1)

    utilities_cpp.write_text(utilities_data)


# 3c. Chemin explicite des helper processes dans l'image Bouchaud.
#
# `application_directory()` vaut /usr/libexec/ladybird pour notre host. Le
# calcul de prefixe generique de Ladybird n'est pas adapte a ce sous-repertoire,
# donc on ajoute le chemin canonique Bouchaud avant les candidats generiques.
utilities_data = utilities_cpp.read_text()
helper_marker = '"/usr/libexec/ladybird/{}"'
if helper_marker not in utilities_data:
    helper_anchor = """    auto application_path = TRY(application_directory());
    Vector<ByteString> paths;
"""
    helper_replacement = """    auto application_path = TRY(application_directory());
    Vector<ByteString> paths;

#if defined(BOUCHAUD_PORT)
    TRY(paths.try_append(ByteString::formatted("/usr/libexec/ladybird/{}", process_name)));
#endif
"""
    if helper_anchor not in utilities_data:
        raise SystemExit("BrowserHost: ancre get_paths_for_helper_process introuvable")
    utilities_cpp.write_text(utilities_data.replace(helper_anchor, helper_replacement, 1))


# 4. WebContent accepte le legacy inherited-fd OU le SOCKET_TAKEOVER upstream.
web_main = root / "Services/WebContent/main.cpp"
data = web_main.read_text()
old = r'''#elif defined(BOUCHAUD_PORT)
    auto* inherited_fd = getenv("BOUCHAUD_WEBCONTENT_FD");
    if (!inherited_fd) {
        warnln("Bouchaud: BOUCHAUD_WEBCONTENT_FD absent");
        return 64;
    }
    auto fd = atoi(inherited_fd);
    if (fd < 0) {
        warnln("Bouchaud: descripteur IPC invalide");
        return 64;
    }
    auto socket = TRY(Core::LocalSocket::adopt_fd(fd));
    auto webcontent_client = WebContent::ConnectionFromClient::construct(make<IPC::Transport>(move(socket)));
    outln("[ladybird-bouchaud] WEBCONTENT_READY pid={} fd={}", Core::System::getpid(), fd);
#else
    auto webcontent_client = TRY(IPC::take_over_accepted_client_from_system_server<WebContent::ConnectionFromClient>(mach_server_name));
#endif'''
new = r'''#elif defined(BOUCHAUD_PORT)
    auto webcontent_client = TRY([&]() -> ErrorOr<NonnullRefPtr<WebContent::ConnectionFromClient>> {
        if (auto* inherited_fd = getenv("BOUCHAUD_WEBCONTENT_FD")) {
            auto fd = atoi(inherited_fd);
            if (fd < 0)
                return Error::from_string_literal("Bouchaud: BOUCHAUD_WEBCONTENT_FD invalide");
            auto socket = TRY(Core::LocalSocket::adopt_fd(fd));
            outln("[ladybird-bouchaud] WEBCONTENT_READY pid={} fd={} mode=legacy-inherited",
                Core::System::getpid(), fd);
            return WebContent::ConnectionFromClient::construct(make<IPC::Transport>(move(socket)));
        }

        auto client = TRY(IPC::take_over_accepted_client_from_system_server<WebContent::ConnectionFromClient>(mach_server_name));
        outln("[ladybird-bouchaud] WEBCONTENT_READY pid={} mode=browser-host-takeover",
            Core::System::getpid());
        return client;
    }());
#else
    auto webcontent_client = TRY(IPC::take_over_accepted_client_from_system_server<WebContent::ConnectionFromClient>(mach_server_name));
#endif'''
if new not in data:
    if old not in data:
        raise SystemExit("BrowserHost: bloc takeover WebContent genere introuvable")
    data = data.replace(old, new, 1)

old_cond = '    if (getenv("BOUCHAUD_M9")) {\n        auto* inherited_request_fd = getenv("BOUCHAUD_REQUEST_FD");'
new_cond = '    if (getenv("BOUCHAUD_M9") && !getenv("BOUCHAUD_BROWSER_HOST")) {\n        auto* inherited_request_fd = getenv("BOUCHAUD_REQUEST_FD");'
if new_cond not in data:
    if old_cond not in data:
        raise SystemExit("BrowserHost: condition ResourceLoader M9 introuvable")
    data = data.replace(old_cond, new_cond, 1)

old_start = '    else if (getenv("BOUCHAUD_M9"))\n        webcontent_client->bouchaud_m9_start();'
new_start = '    else if (getenv("BOUCHAUD_M9") && !getenv("BOUCHAUD_BROWSER_HOST"))\n        webcontent_client->bouchaud_m9_start();'
if new_start not in data:
    if old_start not in data:
        raise SystemExit("BrowserHost: demarrage M9 introuvable")
    data = data.replace(old_start, new_start, 1)

web_main.write_text(data)


# 5. Avec le vrai host, laisser WebContent envoyer cookies/storage/HSTS/workers
# au WebContentClient upstream au lieu des fallbacks temporaires.
page_cpp = root / "Services/WebContent/PageClient.cpp"
ensure_include(page_cpp, "#include <cstdlib>", "#include <AK/JsonObjectSerializer.h>", "cstdlib PageClient")
data = page_cpp.read_text()
old_guard = "if (bouchaud_m9_enabled())"
new_guard = 'if (bouchaud_m9_enabled() && getenv("BOUCHAUD_BROWSER_HOST") == nullptr)'
if new_guard not in data:
    count = data.count(old_guard)
    if count == 0:
        raise SystemExit("BrowserHost: aucun fallback hote local M9 trouve")
    data = data.replace(old_guard, new_guard)
    page_cpp.write_text(data)

# BrowserHost+M11: route les captures du bridge GUI localement.
#
# `queue_screenshot_task()` est utilise par M11 comme mecanisme de
# materialisation CPU d'une frame. Cette capture n'est PAS une reponse a
# `WebView::ViewImplementation::take_screenshot()`. La transmettre au client
# upstream ferait donc tomber `VERIFY(m_pending_screenshot)`.
#
# Garder le VERIFY upstream intact : c'est le protocole du bridge qui doit
# distinguer ses frames des vraies demandes de screenshot.
data = page_cpp.read_text()
screenshot_signature = "void PageClient::page_did_take_screenshot(Gfx::ShareableBitmap const& screenshot)\n{\n"
screenshot_route = """void PageClient::page_did_take_screenshot(Gfx::ShareableBitmap const& screenshot)
{
#if defined(BOUCHAUD_PORT)
    if (getenv(\"BOUCHAUD_BROWSER_HOST\") != nullptr && BouchaudChrome::enabled()) {
        // Le rectangle que `paint_next_frame()` a calcule pour CETTE capture,
        // accumule depuis la precedente. Voir tools/ladybird/prepare-repaint.py :
        // sans lui le chrome recopiait la fenetre entiere a chaque trame, et un
        // curseur qui clignote repeignait 1 554 048 pixels deux fois par
        // seconde.
        auto const degat = page().top_level_traversable()->bouchaud_last_frame_damage();
        if (!BouchaudChrome::present(m_id, screenshot, degat.x(), degat.y(), degat.width(), degat.height()))
            Core::Process::terminate_immediately(70);

        static bool first_frame_reported = false;
        if (!first_frame_reported) {
            first_frame_reported = true;
            outln(\"[ladybird-bouchaud] BROWSER_HOST_M11_FRAME_PRESENTED page={}\", m_id);
        }
        return;
    }
#endif
"""
if screenshot_route not in data:
    if screenshot_signature not in data:
        raise SystemExit("BrowserHost: PageClient::page_did_take_screenshot introuvable")
    data = data.replace(screenshot_signature, screenshot_route, 1)
    page_cpp.write_text(data)


# BrowserHost+M11: les evenements GUI sont injectes directement dans WebContent.
#
# Upstream WebView::ViewImplementation place chaque evenement dans
# `m_pending_input_events` AVANT de l'envoyer a WebContent. Le retour
# `did_finish_handling_input_event()` depile donc cette queue. M11 contourne ce
# chemin : il lit le fd GUI dans WebContent puis appelle les handlers localement.
# Renvoyer l'ACK upstream ferait depiler une queue BrowserHost vide.
data = page_cpp.read_text()
input_ack_old = """void PageClient::report_finished_handling_input_event(u64 page_id, Web::EventResult event_was_handled)
{
    client().async_did_finish_handling_input_event(page_id, event_was_handled);
}"""
input_ack_new = """void PageClient::report_finished_handling_input_event(u64 page_id, Web::EventResult event_was_handled)
{
#if defined(BOUCHAUD_PORT)
    if (getenv(\"BOUCHAUD_BROWSER_HOST\") != nullptr && BouchaudChrome::enabled()) {
        static bool reported_local_input_ack = false;
        if (!reported_local_input_ack) {
            reported_local_input_ack = true;
            outln(\"[ladybird-bouchaud] BROWSER_HOST_M11_INPUT_ACK_LOCAL page={}\", page_id);
        }
        if (BouchaudChrome::wheel_input_pending()) {
            // Le MouseWheel est passe par Page/EventHandler (DOM, overflow,
            // iframe, preventDefault et async scrolling). M11 ne modifie aucun
            // offset: son unique responsabilite supplementaire est de demander
            // le readback du Compositor, puisque sa fenetre Bouchaud presente
            // des screenshots et non la surface native du Compositor.
            BouchaudChrome::wheel_handled_and_capture_requested(static_cast<int>(event_was_handled));
            outln(\"[ladybird-bouchaud] WEB_SCREENSHOT_REQUEST after_wheel=1 page={}\", page_id);
            page().top_level_traversable()->bouchaud_schedule_interactive_frame_capture();
        }
        return;
    }
#endif
    client().async_did_finish_handling_input_event(page_id, event_was_handled);
}"""
if input_ack_new not in data:
    if input_ack_old not in data:
        raise SystemExit("BrowserHost: report_finished_handling_input_event introuvable")
    data = data.replace(input_ack_old, input_ack_new, 1)
    page_cpp.write_text(data)


# `prepare-console.py` installe aujourd'hui la sortie serie sous le test M9
# direct. Une ancienne revision a aussi produit une variante qui excluait le
# BrowserHost. Accepter les deux garde le preparateur compatible avec les caches
# locaux/CI tout en conservant une erreur franche si le contrat derive vraiment.
data = page_cpp.read_text()
console_prepare = """    if (bouchaud_m9_enabled()) {
        console_output.output.visit("""
console_legacy = """    if (bouchaud_m9_enabled() && getenv("BOUCHAUD_BROWSER_HOST") == nullptr) {
        console_output.output.visit("""
console_new = """    if (bouchaud_m9_enabled() || getenv("BOUCHAUD_BROWSER_HOST") != nullptr) {
        console_output.output.visit("""
if console_new not in data:
    for console_old in (console_prepare, console_legacy):
        if console_old in data:
            page_cpp.write_text(data.replace(console_old, console_new, 1))
            break
    else:
        raise SystemExit("BrowserHost: passerelle console prepare-console introuvable")


# 6. Le vrai Compositor est actif uniquement en mode BrowserHost.
page_client_h = root / "Services/WebContent/PageClient.h"
ensure_include(page_client_h, "#include <cstdlib>", "#pragma once", "cstdlib PageClient.h")
replace_once(
    page_client_h,
    '''#if defined(BOUCHAUD_PORT)
    virtual bool supports_compositor() const override { return false; }
#else
    virtual bool supports_compositor() const override { return true; }
#endif''',
    '''#if defined(BOUCHAUD_PORT)
    virtual bool supports_compositor() const override { return getenv("BOUCHAUD_BROWSER_HOST") != nullptr; }
#else
    virtual bool supports_compositor() const override { return true; }
#endif''',
    "supports_compositor runtime",
)


# 7. M11 s'attache a la page creee par Application au lieu d'en creer une autre.
#
# BOUCHAUD_C22_ONGLETS : `bouchaud_m11_start()` ne prend plus d'identifiant de
# page. Il en prenait un parce qu'il n'y en avait qu'un et qu'il fallait bien le
# choisir ; depuis les onglets, chaque rappel demande au chrome quel onglet est
# ACTIF, et un identifiant fige a l'attache serait justement le defaut qu'on
# cherche a eviter. Ce qui reste a faire ici est d'enregistrer la page de
# depart comme premier onglet.
connection_cpp = root / "Services/WebContent/ConnectionFromClient.cpp"
data = connection_cpp.read_text()

old_init = '''void ConnectionFromClient::initialize(u64 initial_page_id, Web::HTML::CrossProcessId root_navigable_id, Web::HTML::CrossProcessIdAllocator cross_process_id_allocator)
{
    m_page_host->initialize(initial_page_id, root_navigable_id, cross_process_id_allocator);
}'''
new_init = '''void ConnectionFromClient::initialize(u64 initial_page_id, Web::HTML::CrossProcessId root_navigable_id, Web::HTML::CrossProcessIdAllocator cross_process_id_allocator)
{
    m_page_host->initialize(initial_page_id, root_navigable_id, cross_process_id_allocator);

#if defined(BOUCHAUD_PORT)
    if (getenv("BOUCHAUD_BROWSER_HOST") && getenv("BOUCHAUD_M11")) {
        BouchaudChrome::initialize_from_environment();
        auto* requested_url = getenv("BOUCHAUD_M9_URL");
        auto const url_de_depart = (requested_url && *requested_url)
            ? ByteString { requested_url }
            : ByteString { "about:blank" };
        // La page que `Application` vient de creer devient le premier onglet.
        // Elle est enregistree AVANT toute annonce d'URL : sans onglet,
        // `page_active()` repond par defaut et le chrome rangerait l'etat de
        // cette page dans un onglet qui n'existe pas.
        BouchaudChrome::ajoute_onglet(initial_page_id, url_de_depart, true);
        BouchaudChrome::set_committed_url(initial_page_id, url_de_depart);
        bouchaud_m11_start();
        outln("[ladybird-bouchaud] BROWSER_HOST_M11_ATTACHED page={}", initial_page_id);
    }
#endif
}'''
if new_init not in data:
    if old_init not in data:
        raise SystemExit("BrowserHost: ConnectionFromClient::initialize introuvable")
    data = data.replace(old_init, new_init, 1)

connection_cpp.write_text(data)

# ---------------------------------------------------------------------------
# 9. Le cycle de vie d'un WebWorker, dit etape par etape.
# ---------------------------------------------------------------------------
#
# BOUCHAUD_C26_CYCLE_DE_VIE_DU_WORKER
#
# Le smoke test du run 35742940872 echoue sur un seul point, et il ne dit
# qu'une chose :
#
#     HOST_WORKER FAIL Error: worker timeout
#     HOST_SMOKE_FAIL canvas=1 worker=0 image=1 frame=1
#
# « timeout » ne distingue pas les cinq pannes possibles, et chacune a un
# remede different : la vue n'est pas trouvee cote hote ; le processus n'est
# pas lance ; il est lance mais son IPC n'arrive jamais ; il repond mais ne
# charge pas son script ; il charge son script et le message ne revient pas.
#
# Le journal serie ne portait AUCUNE ligne de worker -- ni lancement, ni
# refus, ni erreur. Un chemin entierement muet ne se diagnostique pas a
# distance, et chaque aller-retour de CI coute une dizaine de minutes.
#
# Les traces ci-dessous sont donc posees aux QUATRE frontieres que la revue
# demande de distinguer : la requete recue, le processus lance, les services
# frere raccordes, et le verdict rendu a WebContent.

# L'HORLOGE DES TRACES DE WORKER.
#
# BOUCHAUD_C29_WORKER_HORODATE
#
# Les dix traces `WORKER_ETAPE` existaient deja, mais AUCUNE ne portait
# d'instant. Elles disaient donc quelles etapes avaient ete franchies, et
# rien sur le temps passe entre elles -- or c'est exactement la question :
# le premier worker met plus de deux minutes, et il faut savoir OU.
#
# `MonotonicTime::now().milliseconds()` rend un instant monotone ABSOLU, et
# c'est ce qui compte ici : les etapes sont emises par TROIS processus
# differents -- l'hote, le worker, WebContent. Un temps relatif au demarrage
# de chacun ne serait pas comparable d'une ligne a l'autre ; une horloge
# commune permet de soustraire.
#
# `AK/Time.h` n'est inclus par aucun des trois : l'oubli ne se verrait qu'au
# bout de douze minutes de compilation.
def ajoute_include_time(chemin, ancre):
    """Garantit `#include <AK/Time.h>` dans un fichier qui horodate."""
    texte = chemin.read_text()
    if "#include <AK/Time.h>" in texte:
        return
    if ancre not in texte:
        raise SystemExit(f"horodatage : ancre d'inclusion introuvable dans {chemin}")
    chemin.write_text(texte.replace(ancre, "#include <AK/Time.h>\n" + ancre, 1))


worker_manager = root / "Libraries/LibWebView/WorkerProcessManager.cpp"
ajoute_include_time(worker_manager, "#include <LibCore/EventLoop.h>\n")
ensure_include(
    worker_manager,
    "#if defined(BOUCHAUD_PORT)\n#    include <LibCore/System.h>\n#endif",
    "#include <LibWebView/WorkerProcessManager.h>",
    "LibCore/System WorkerProcessManager",
)

replace_once(
    worker_manager,
    """    auto agent_id = ++m_next_agent_id;
    auto client = MUST(launch_web_worker_process(request.agent_type, is_private, agent_id));

    auto request_server_handle = MUST(connect_new_request_server_client(is_private));
    auto image_decoder_handle = MUST(connect_new_image_decoder_client());
    client->async_connect_to_request_server(move(request_server_handle));
    client->async_connect_to_image_decoder(move(image_decoder_handle));""",
    """    auto agent_id = ++m_next_agent_id;
#if defined(BOUCHAUD_PORT)
    // Chaque etape porte l'identifiant de l'agent : plusieurs workers peuvent
    // demarrer en meme temps, et leurs lignes s'entrelaceraient.
    outln("[ladybird-bouchaud] WORKER_ETAPE t={} agent={} etape=lancement_demande url={}", MonotonicTime::now().milliseconds(), agent_id, request.url);
#endif
    auto client = MUST(launch_web_worker_process(request.agent_type, is_private, agent_id));
#if defined(BOUCHAUD_PORT)
    outln("[ladybird-bouchaud] WORKER_ETAPE t={} agent={} etape=processus_lance pid={}", MonotonicTime::now().milliseconds(), agent_id, client->pid());
#endif

    // LES TROIS RACCORDEMENTS SONT TRACES SEPAREMENT, ET CE N'EST PAS DU ZELE.
    //
    // Les deux premiers sont des `MUST` : un echec y termine le processus
    // HOTE, c'est-a-dire tout le navigateur. Le troisieme est tolere. Savoir
    // lequel des trois a ete franchi est la difference entre « le worker
    // n'est pas parti » et « le navigateur est mort en essayant ».
    auto request_server_handle = MUST(connect_new_request_server_client(is_private));
#if defined(BOUCHAUD_PORT)
    outln("[ladybird-bouchaud] WORKER_ETAPE t={} agent={} etape=request_server_raccorde", MonotonicTime::now().milliseconds(), agent_id);
#endif
    auto image_decoder_handle = MUST(connect_new_image_decoder_client());
#if defined(BOUCHAUD_PORT)
    outln("[ladybird-bouchaud] WORKER_ETAPE t={} agent={} etape=image_decoder_raccorde", MonotonicTime::now().milliseconds(), agent_id);
#endif
    client->async_connect_to_request_server(move(request_server_handle));
    client->async_connect_to_image_decoder(move(image_decoder_handle));""",
    "traces de lancement du worker",
)

replace_once(
    worker_manager,
    """void WorkerProcessManager::notify_worker_script_load_success(Owner const& owner)
{""",
    """void WorkerProcessManager::notify_worker_script_load_success(Owner const& owner)
{
#if defined(BOUCHAUD_PORT)
    outln("[ladybird-bouchaud] WORKER_ETAPE t={} etape=script_charge", MonotonicTime::now().milliseconds());
#endif""",
    "trace de chargement reussi",
)

replace_once(
    worker_manager,
    """void WorkerProcessManager::notify_worker_script_load_failure(Owner const& owner)
{""",
    """void WorkerProcessManager::notify_worker_script_load_failure(Owner const& owner)
{
#if defined(BOUCHAUD_PORT)
    // L'ECHEC DE CHARGEMENT EST LA PANNE LA PLUS PROBABLE POUR UNE URL blob:.
    //
    // Un `blob:` appartient a l'agent qui l'a cree ; un WebWorker Ladybird est
    // un processus separe. Le resoudre demande que le magasin d'URL de blob
    // traverse la frontiere de processus, ce qui n'a rien a voir avec le
    // lancement du processus lui-meme.
    outln("[ladybird-bouchaud] WORKER_ETAPE t={} etape=script_echoue", MonotonicTime::now().milliseconds());
#endif""",
    "trace de chargement echoue",
)


# Le processus WebWorker lui-meme : sa naissance et son IPC.
worker_main = root / "Services/WebWorker/main.cpp"
ajoute_include_time(worker_main, "#include <LibCore/ArgsParser.h>\n")
replace_once(
    worker_main,
    """    auto client = TRY(IPC::take_over_accepted_client_from_system_server<WebWorker::ConnectionFromClient>(mach_server_name));""",
    """#if defined(BOUCHAUD_PORT)
    outln("[ladybird-bouchaud] WORKER_ETAPE t={} etape=main pid={}", MonotonicTime::now().milliseconds(), Core::System::getpid());
#endif
    auto client = TRY(IPC::take_over_accepted_client_from_system_server<WebWorker::ConnectionFromClient>(mach_server_name));
#if defined(BOUCHAUD_PORT)
    // L'IPC EST PRET, et c'est un fait distinct du fait que le processus
    // tourne. Un WebWorker lance dont le transport n'arrive jamais et un
    // WebWorker jamais lance se ressemblent vus de la page : les deux donnent
    // « timeout ».
    outln("[ladybird-bouchaud] WORKER_ETAPE t={} etape=ipc_pret pid={}", MonotonicTime::now().milliseconds(), Core::System::getpid());
#endif""",
    "traces de naissance du WebWorker",
)
replace_once(
    worker_main,
    """    return event_loop.exec();""",
    """#if defined(BOUCHAUD_PORT)
    // LA BOUCLE D'EVENEMENTS TOURNE, et c'est un troisieme fait distinct.
    //
    // `ipc_pret` dit que le transport est adopte ; il ne dit pas que le
    // processus est en mesure de SERVIR. Entre les deux il reste l'edition de
    // liens paresseuse, l'initialisation d'ICU et celle de la machine
    // JavaScript. C'est le dernier intervalle avant que le worker ne puisse
    // recevoir un message, et donc le dernier endroit ou un demarrage a froid
    // peut se cacher sans etre attribue.
    outln("[ladybird-bouchaud] WORKER_ETAPE t={} etape=boucle_prete pid={}", MonotonicTime::now().milliseconds(), Core::System::getpid());
#endif
    return event_loop.exec();""",
    "trace de boucle d'evenements du WebWorker",
)
ensure_include(
    worker_main,
    "#include <LibCore/System.h>",
    "#include <LibMain/Main.h>",
    "LibCore/System WebWorker main",
)


# BOUCHAUD_V13_PROC_STAT_REOPEN
#
# `ProcessStatisticsLinux.cpp` conserve normalement `/proc/stat` ouvert dans un
# `NeverDestroyed<Core::File>` et fait `seek(0)` entre deux mesures. Sur Linux,
# procfs regenere le contenu a chaque lecture. Sur Bouchaud, nos pseudo-fichiers
# `/proc` sont des instantanes fabriques a `open(2)` (meme contrat que
# `/proc/self/maps`) : seek(0) relirait donc une photo ancienne pour toujours.
# Rouvrir le fichier a chaque echantillon rend le contrat equivalent a procfs
# sans ajouter un deuxieme type de descripteur dynamique au noyau.
process_stats_linux = root / "Libraries/LibCore/Platform/ProcessStatisticsLinux.cpp"
replace_once(
    process_stats_linux,
    '''    static NeverDestroyed<NonnullOwnPtr<Core::File>> proc_stat { TRY(Core::File::open("/proc/stat"sv, Core::File::OpenMode::Read)) };
    TRY((*proc_stat)->seek(0, SeekMode::SetPosition));

    char buf[1024] = {};
    auto buffer = Bytes { buf, sizeof(buf) };
    auto line = TRY((*proc_stat)->read_some(buffer));''',
    '''#if defined(BOUCHAUD_PORT)
    // BOUCHAUD_V13_PROC_STAT_REOPEN
    // Bouchaud /proc est snapshot-per-open : un seek(0) ne regenere pas le
    // contenu. Reouvrir est donc l'equivalent exact de la lecture procfs Linux.
    auto proc_stat = TRY(Core::File::open("/proc/stat"sv, Core::File::OpenMode::Read));
#else
    static NeverDestroyed<NonnullOwnPtr<Core::File>> proc_stat { TRY(Core::File::open("/proc/stat"sv, Core::File::OpenMode::Read)) };
    TRY((*proc_stat)->seek(0, SeekMode::SetPosition));
#endif

    char buf[1024] = {};
    auto buffer = Bytes { buf, sizeof(buf) };
#if defined(BOUCHAUD_PORT)
    auto line = TRY(proc_stat->read_some(buffer));
#else
    auto line = TRY((*proc_stat)->read_some(buffer));
#endif''',
    "Bouchaud /proc/stat snapshot reopen",
)

print("Browser Host phase 1 applique au worktree:", root)
print(" - WebView::Application upstream")
print(" - RequestServer/ImageDecoder/Compositor upstream")
print(" - WebWorker upstream a la demande")
print(" - M11 conserve comme bridge GUI temporaire")

# ---------------------------------------------------------------------------
# BOUCHAUD_M11_TAB_HOST_TRACE_V1
# ---------------------------------------------------------------------------
webcontent_client_cpp = root / "Libraries/LibWebView/WebContentClient.cpp"
ajoute_include_time(webcontent_client_cpp, "#include <AK/Debug.h>\n")

replace_once(
    webcontent_client_cpp,
    """    view.m_client_state.page_index = page_id;
    m_views.set(page_id, view);
    m_history_recorded_urls_for_current_load.remove(page_id);
}""",
    """    view.m_client_state.page_index = page_id;
    m_views.set(page_id, view);
#if defined(BOUCHAUD_PORT)
    outln("[ladybird-bouchaud] M11_HOST_TAB_STAGE 40 VIEW_REGISTERED page={}", page_id);
#endif
    m_history_recorded_urls_for_current_load.remove(page_id);
}""",
    "M11 host register_view",
)

replace_once(
    webcontent_client_cpp,
    """Messages::WebContentClient::DidRequestNewWebViewResponse WebContentClient::did_request_new_web_view(u64 page_id, Web::HTML::ActivateTab activate_tab, Web::HTML::WebViewHints hints)
{
    auto new_page_id = Application::the().allocate_page_id();
    String handle;
    if (auto view = view_for_page_id(page_id); view.has_value()) {
        if (view->on_new_web_view)
            handle = view->on_new_web_view(activate_tab, hints, new_page_id);
    }

    auto view = view_for_page_id(new_page_id);
    if (!view.has_value())
        return { {}, {}, move(handle) };

    auto root_navigable_id = Application::the().allocate_ui_process_cross_process_id();
    view->traversable().set_id(root_navigable_id);

    return { new_page_id, root_navigable_id, move(handle) };
}""",
    """Messages::WebContentClient::DidRequestNewWebViewResponse WebContentClient::did_request_new_web_view(u64 page_id, Web::HTML::ActivateTab activate_tab, Web::HTML::WebViewHints hints)
{
#if defined(BOUCHAUD_PORT)
    outln("[ladybird-bouchaud] M11_HOST_TAB_STAGE 10 REQUEST_RECEIVED source={}", page_id);
#endif
    auto new_page_id = Application::the().allocate_page_id();
#if defined(BOUCHAUD_PORT)
    outln("[ladybird-bouchaud] M11_HOST_TAB_STAGE 20 PAGE_ALLOCATED source={} page={}", page_id, new_page_id);
#endif
    String handle;
    if (auto view = view_for_page_id(page_id); view.has_value()) {
        if (view->on_new_web_view)
            handle = view->on_new_web_view(activate_tab, hints, new_page_id);
    }
#if defined(BOUCHAUD_PORT)
    outln("[ladybird-bouchaud] M11_HOST_TAB_STAGE 30 CHILD_CALLBACK_DONE source={} page={}", page_id, new_page_id);
#endif

    auto view = view_for_page_id(new_page_id);
    if (!view.has_value()) {
#if defined(BOUCHAUD_PORT)
        warnln("[ladybird-bouchaud] M11_HOST_TAB_STAGE 41 VIEW_MISSING source={} page={}", page_id, new_page_id);
#endif
        return { {}, {}, move(handle) };
    }

#if defined(BOUCHAUD_PORT)
    outln("[ladybird-bouchaud] M11_HOST_TAB_STAGE 50 VIEW_LOOKUP_OK source={} page={}", page_id, new_page_id);
#endif
    auto root_navigable_id = Application::the().allocate_ui_process_cross_process_id();
    view->traversable().set_id(root_navigable_id);
#if defined(BOUCHAUD_PORT)
    outln("[ladybird-bouchaud] M11_HOST_TAB_STAGE 60 REPLY source={} page={}", page_id, new_page_id);
#endif
    return { new_page_id, root_navigable_id, move(handle) };
}""",
    "M11 host DidRequestNewWebView",
)


# La requete elle-meme, au moment ou l'hote la recoit. C'est la premiere
# frontiere, et celle qui rend `{0}` silencieusement.
replace_once(
    webcontent_client_cpp,
    """Messages::WebContentClient::StartWorkerAgentResponse WebContentClient::start_worker_agent(u64 page_id, Web::HTML::WorkerAgentStartRequest request)
{
    if (auto view = view_for_page_id(page_id); view.has_value()) {
        auto agent_id = WorkerProcessManager::the().start_worker_agent(*this, page_id, move(request));
        return { agent_id };
    }

    return { 0 };
}""",
    """Messages::WebContentClient::StartWorkerAgentResponse WebContentClient::start_worker_agent(u64 page_id, Web::HTML::WorkerAgentStartRequest request)
{
    if (auto view = view_for_page_id(page_id); view.has_value()) {
        auto agent_id = WorkerProcessManager::the().start_worker_agent(*this, page_id, move(request));
#if defined(BOUCHAUD_PORT)
        outln("[ladybird-bouchaud] WORKER_ETAPE t={} etape=agent_rendu page_id={} agent={}", MonotonicTime::now().milliseconds(), page_id, agent_id);
#endif
        return { agent_id };
    }

#if defined(BOUCHAUD_PORT)
    // LE REFUS SILENCIEUX.
    //
    // Upstream rend `{0}` sans un mot quand la page n'est pas dans le
    // registre de vues. Cote WebContent, un agent zero se lit « pas de
    // worker », et la page attend son message jusqu'a expiration du garde --
    // ce qui donne exactement « worker timeout », sans aucune trace.
    outln("[ladybird-bouchaud] WORKER_ETAPE t={} etape=refuse_page_inconnue page_id={} vues={}", MonotonicTime::now().milliseconds(), page_id, m_views.size());
#endif
    return { 0 };
}""",
    "trace de refus du worker",
)
