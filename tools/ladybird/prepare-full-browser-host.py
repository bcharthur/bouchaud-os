#!/usr/bin/env python3
'''
Couche plateforme du processus navigateur Bouchaud (LibCore, LibWebView).

S'applique au worktree Ladybird jetable apres les adaptations existantes.

Cible:
    Bouchaud WM
      -> BouchaudBrowserHost (UI/Bouchaud, voir prepare-ui-bouchaud.py)
          -> RequestServer
          -> ImageDecoder
          -> Compositor (--force-cpu-painting)
          -> WebContent (contenu seulement : ni chrome, ni fenetre)
          -> WebWorker(s) a la demande via WorkerProcessManager

Ce script ne cree plus l'hote et ne touche plus WebContent : il porte ce que
Bouchaud demande a LibCore/LibWebView (pas de PR_SET_PDEATHSIG, pas de /proc,
racine des ressources, chemin des services, /proc/stat relu) et les traces du
cycle de vie des WebWorkers. BOUCHAUD_UI_V1.
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


def ajoute_include_time(chemin, ancre):
    """Garantit `#include <AK/Time.h>` dans un fichier qui horodate."""
    texte = chemin.read_text()
    if "#include <AK/Time.h>" in texte:
        return
    if ancre not in texte:
        raise SystemExit(f"horodatage : ancre d'inclusion introuvable dans {chemin}")
    chemin.write_text(texte.replace(ancre, "#include <AK/Time.h>\n" + ancre, 1))


# 2. Core::Process: garder le vrai fork/exec upstream, mais Bouchaud ne fournit
# pas encore PR_SET_PDEATHSIG.
process_cpp = root / "Libraries/LibCore/Process.cpp"
if "BOUCHAUD_PORT: pas encore de PR_SET_PDEATHSIG" not in process_cpp.read_text():
    # Un fichier .cpp inclut toujours son propre en-tete : l'ancre est sure.
    ajoute_include_time(process_cpp, "#include <LibCore/Process.h>\n")
    # `outln` n'est pas garanti visible ici : LibCore/Process.cpp n'ecrit rien
    # en amont. Un oubli ne se verrait qu'au bout de douze minutes de
    # compilation, et pour une seule ligne de trace.
    ensure_include(
        process_cpp,
        "#include <AK/Format.h>",
        "#include <LibCore/Process.h>",
        "AK/Format Process.cpp",
    )
    replace_once(
        process_cpp,
        '''    auto parent_pid = getpid();
    auto pid = fork();''',
        '''#if !defined(BOUCHAUD_PORT)
    auto parent_pid = getpid();
#endif
#if defined(BOUCHAUD_PORT)
    // LE LANCEMENT DE PROCESSUS, BORNE DES DEUX COTES, POUR TOUS LES SERVICES.
    //
    // BOUCHAUD_C34_TOUS_LES_SERVICES
    //
    // Les etapes de worker ne bornaient qu'un seul des six processus du
    // navigateur. BrowserHost, RequestServer, ImageDecoder, Compositor et
    // WebContent passent tous par ICI -- `Core::Process::spawn` est
    // l'entonnoir commun -- et aucun n'avait de mesure de son lancement.
    //
    // Le `fork` est isole du reste parce que son cout n'est pas le meme
    // nature : le noyau recopie l'espace d'adressage du pere page par page
    // (voir `PERF_FORK`, environ deux millisecondes par mebioctet resident),
    // alors que l'`execve` qui suit et le raccordement du transport sont des
    // couts fixes. Un chiffre unique melangerait les deux.
    auto bouchaud_avant_fork = MonotonicTime::now().milliseconds();
#endif
    auto pid = fork();
#if defined(BOUCHAUD_PORT)
    if (pid > 0) {
        auto bouchaud_apres_fork = MonotonicTime::now().milliseconds();
        outln("[ladybird-bouchaud] SPAWN_ETAPE t={} etape=fork_rendu enfant={} debut={} fork_ms={}",
            bouchaud_apres_fork, pid, bouchaud_avant_fork,
            bouchaud_apres_fork - bouchaud_avant_fork);
    }
#endif''',
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
print(" - le frontend (UI/Bouchaud) est installe par prepare-ui-bouchaud.py")

webcontent_client_cpp = root / "Libraries/LibWebView/WebContentClient.cpp"
ajoute_include_time(webcontent_client_cpp, "#include <AK/Debug.h>\n")

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
#if defined(BOUCHAUD_PORT)
    // LE PREMIER INSTANT DE LA CHAINE, et il manquait.
    //
    // BOUCHAUD_C29_WORKER_PREMIERE_DEMANDE
    //
    // Le releve du run 35829303875 porte les etapes des agents 2, 3 et 4 --
    // et AUCUNE pour le premier. Son processus a pourtant trace sa propre
    // naissance (`main pid=16`), si bien que le cout le plus interessant de
    // tous, celui du demarrage a froid, est le seul sans borne de depart.
    //
    // `lancement_demande` est emis par WorkerProcessManager, plus loin dans
    // la chaine. Cette ligne-ci est emise a l'ARRIVEE de la demande, avant
    // toute recherche de vue et avant tout refus : elle ne peut pas manquer.
    outln("[ladybird-bouchaud] WORKER_ETAPE t={} etape=demande_recue page_id={}",
        MonotonicTime::now().milliseconds(), page_id);
#endif
    if (auto view = view_for_page_id(page_id); view.has_value()) {
#if defined(BOUCHAUD_PORT)
        // La recherche de vue separe `demande_recue` de tout le reste. Elle
        // est probablement instantanee -- mais « probablement » n'est pas une
        // mesure, et c'est le seul intervalle de cette fonction qui n'en avait
        // aucune.
        outln("[ladybird-bouchaud] WORKER_ETAPE t={} etape=vue_trouvee page_id={}", MonotonicTime::now().milliseconds(), page_id);
#endif
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
