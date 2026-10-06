/*
 * Bouchaud OS -- UI/Bouchaud : point d'entree du navigateur.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 *
 * BOUCHAUD_UI_V1
 *
 * Le binaire garde son nom historique, `BouchaudBrowserHost` : le noyau le
 * lance sous ce nom (`src/kernel/security/profile.rs`, supervision,
 * `/bo-navigateur`). Ce n'est plus un hote headless qui laisse WebContent
 * dessiner sa propre fenetre : c'est le processus NAVIGATEUR, au sens
 * d'upstream. Il tient la fenetre, le chrome, les onglets, le presse-papiers,
 * les telechargements ; WebContent ne fait que du contenu.
 */

#include <AK/ByteString.h>
#include <AK/StringView.h>
#include <AK/Vector.h>
#include <LibHTTP/Cache/Utilities.h>
#include <LibMain/Main.h>
#include <LibURL/URL.h>
#include <LibWebView/Settings.h>
#include <UI/Bouchaud/Application.h>

#include <cstdlib>
#include <cstring>
#include <fcntl.h>
#include <sys/stat.h>
#include <unistd.h>

namespace {

int env_dimension(char const* name, int fallback)
{
    auto* value = getenv(name);
    if (!value || !*value)
        return fallback;
    auto parsed = atoi(value);
    return parsed > 0 ? parsed : fallback;
}

char const* env_ou(char const* name, char const* fallback)
{
    auto* value = getenv(name);
    return (value && *value) ? value : fallback;
}

// Le cache HTTP est REGENERABLE, et il le dit : `CACHEDIR.TAG` (Cache
// Directory Tagging Specification). Sous `/persist`, la persistance du noyau
// ecarte un arbre ainsi etiquete, en entier, quand la zone ne le tient plus --
// au lieu de faire echouer la sauvegarde des cookies et des reglages avec lui
// (`src/fs/cache_jetable.rs`, BOUCHAUD_PERSIST_CACHE_JETABLE_V1).
bool etiquette_le_cache(char const* dossier)
{
    static constexpr char contenu[] = "Signature: 8a477f597d28d172789f06886806bc55\n"
                                      "# Cache HTTP de Ladybird (Bouchaud OS). Regenerable : la persistance l'ecarte\n"
                                      "# en entier quand /persist ne tient plus dans sa zone.\n";
    auto chemin = ByteString::formatted("{}/CACHEDIR.TAG", dossier);
    struct stat etat {};
    if (stat(chemin.characters(), &etat) == 0 && etat.st_size >= 43)
        return true;
    int fd = open(chemin.characters(), O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0600);
    if (fd < 0)
        return false;
    auto const taille = strlen(contenu);
    bool const ok = write(fd, contenu, taille) == static_cast<ssize_t>(taille);
    close(fd);
    return ok;
}

// Le plafond du cache HTTP sur disque. Upstream le derive de l'espace libre
// (`statvfs`), jusqu'a 5 Gio. Deux bornes Bouchaud ne se voient pas dans ce
// calcul : en mode ephemere le « disque » est la RAM (un cache de 20 % de la
// memoire libre serait de la memoire prise aux pages), et le RAMFS n'a que
// `MAX_NODES` = 4096 inodes pour TOUT le systeme -- un fichier par reponse.
// Sous `/persist`, `statfs` annonce deja la zone (64 Mio) ; 32 Mio bornent
// les deux cas. Un reglage choisi par l'utilisateur (about:settings) n'est
// pas ecrase : seul le defaut upstream l'est.
constexpr u64 PLAFOND_CACHE_HTTP = 32 * MiB;

Main::Arguments make_arguments(Vector<ByteString>& storage, Vector<char*>& argv, Vector<StringView>& strings)
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
    mkdir("/tmp/ladybird-runtime", 0700);
    mkdir("/tmp/fontconfig", 0777);
    chmod("/tmp/ladybird-runtime", 0700);

    // Le profil. `/persist` est monte par Bouchaud avant l'autorun ; le mode
    // ephemere (bureau live, banc) garde tout en RAM.
    //
    // BOUCHAUD_PROFIL_XDG_V1 : plus de `--profile-path`. Upstream y range
    // config, donnees, cache ET runtime sous une seule racine -- les sockets
    // et le fichier pid auraient donc vecu dans `/persist`, et survecu a un
    // redemarrage. Sans selecteur, upstream prend le profil nomme `default`
    // sous les racines XDG : `/persist/ladybird/{config,data,cache}/Ladybird/
    // Profiles/default`, et le runtime sous `XDG_RUNTIME_DIR` (`/tmp`).
    bool const ephemere = getenv("BOUCHAUD_LADYBIRD_EPHEMERAL") != nullptr;
    char const* home = ephemere ? "/tmp/ladybird" : "/persist/ladybird";
    char const* config = ephemere ? "/tmp/ladybird-config" : "/persist/ladybird/config";
    char const* donnees = ephemere ? "/tmp/ladybird-data" : "/persist/ladybird/data";
    char const* cache = ephemere ? "/tmp/ladybird-cache" : "/persist/ladybird/cache";
    char const* telechargements = ephemere ? "/tmp" : "/persist/Downloads";
    if (!ephemere)
        mkdir("/persist/ladybird", 0700);
    for (auto* dossier : { home, config, donnees, cache })
        mkdir(dossier, 0700);
    if (!ephemere)
        mkdir("/persist/Downloads", 0755);
    bool const cache_etiquete = etiquette_le_cache(cache);

    setenv("HOME", home, 1);
    setenv("XDG_CONFIG_HOME", config, 1);
    setenv("XDG_RUNTIME_DIR", "/tmp/ladybird-runtime", 1);
    setenv("XDG_DATA_HOME", donnees, 1);
    setenv("XDG_CACHE_HOME", cache, 1);
    setenv("XDG_DOWNLOAD_DIR", telechargements, 1);

    // L'audio : LibMedia joue dans WebContent par `PlaybackStreamBouchaud`
    // (/dev/dsp, OSS -- BOUCHAUD_AUDIO_DSP_V1). `BOUCHAUD_DISABLE_AUDIO`, herite
    // par WebContent, force le repli upstream sur la sortie nulle. Les
    // variables SDL posees ici auparavant n'etaient lues par personne.

    constexpr char fontconfig_file[] = "/usr/share/ladybird/fontconfig/fonts.conf";
    if (access(fontconfig_file, R_OK) == 0) {
        setenv("FONTCONFIG_FILE", fontconfig_file, 1);
        setenv("FONTCONFIG_PATH", "/usr/share/ladybird/fontconfig", 1);
    }

    auto const largeur = env_dimension("BO_SURFACE_WIDTH", 1100);
    auto const hauteur = env_dimension("BO_SURFACE_HEIGHT", 604);
    char const* url = env_ou("BOUCHAUD_M9_URL", "https://example.com/");
    char const* dns = env_ou("BOUCHAUD_DNS_SERVER", "10.0.2.3");
    // Le chrome vit dans ce processus : aucun WebContent ne touche la
    // fenetre, et un echange de processus a la navigation inter-sites est
    // donc sans danger pour elle. L'isolation upstream par defaut s'applique.
    char const* isolation = env_ou("BOUCHAUD_SITE_ISOLATION", "top-level");
    char const* fuseau = env_ou("BOUCHAUD_TIME_ZONE", "Europe/Paris");

    Vector<ByteString> arguments;
    arguments.append("BouchaudBrowserHost");
    arguments.append("--force-cpu-painting");
    arguments.append("--force-fontconfig");
    // Pas de `--disable-sandbox` : BOUCHAUD_SANDBOX_V1. Le noyau confine
    // chaque service par profil (`src/kernel/security/profile.rs`) et chaque
    // service le verifie avant de traiter une donnee du reseau
    // (`tools/ladybird/sandbox/BouchaudConfinement.h`). Le drapeau reste
    // disponible pour un diagnostic, jamais par defaut.
    if (getenv("BOUCHAUD_DISABLE_SANDBOX"))
        arguments.append("--disable-sandbox");
    if (getenv("BOUCHAUD_DISABLE_DISK_CACHE"))
        arguments.append("--disable-http-disk-cache");
    if (getenv("BOUCHAUD_DISABLE_SQL"))
        arguments.append("--disable-sql-database");
    if (getenv("BOUCHAUD_DISABLE_ASYNC_SCROLLING"))
        arguments.append("--disable-async-scrolling");
    arguments.append("--site-isolation");
    arguments.append(isolation);
    if (getenv("BOUCHAUD_ALLOW_POPUPS"))
        arguments.append("--allow-popups");
    if (getenv("BOUCHAUD_ENABLE_AUTOPLAY"))
        arguments.append("--enable-autoplay");
    arguments.append("--default-time-zone");
    arguments.append(fuseau);
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

    warnln("[ladybird-bouchaud] BROWSER_HOST_START architecture=UI/Bouchaud");
    warnln("BOUCHAUD_UI_CHROME_OWNER browser");
    warnln("BOUCHAUD_UI_WEBCONTENT_CHROME 0");
    warnln("[LB:UI] surface={}x{} url={}", largeur, hauteur, url);
    warnln("[LB:UI] profil persistant={} config={} donnees={} cache={} telechargements={} ressources=/usr/share/ladybird",
        ephemere ? "non"sv : "oui"sv, config, donnees, cache, telechargements);
    warnln("[LB:UI] plateforme sandbox={} sql={} cache_disque={} defilement_async={} isolation={} fuseau={} audio={}",
        getenv("BOUCHAUD_DISABLE_SANDBOX") ? "DESACTIVE"sv : "noyau+verification"sv,
        getenv("BOUCHAUD_DISABLE_SQL") ? "non"sv : "oui"sv,
        getenv("BOUCHAUD_DISABLE_DISK_CACHE") ? "non"sv : "oui"sv,
        getenv("BOUCHAUD_DISABLE_ASYNC_SCROLLING") ? "non"sv : "oui"sv,
        isolation, fuseau,
        getenv("BOUCHAUD_DISABLE_AUDIO") ? "non"sv : "/dev/dsp"sv);

    // Trois etapes distinctes, et chacune se dit : un processus qui disparait
    // en silence oblige a deviner OU il s'est arrete.
    int code = 0;
    {
        auto application = TRY(BouchaudUI::Application::create(host_arguments));
        warnln("[ladybird-bouchaud] BROWSER_HOST_INITIALIZED");

        auto& reglages = WebView::Application::settings();
        auto navigation = reglages.browsing_data_settings();
        if (navigation.disk_cache_settings.maximum_size == HTTP::DEFAULT_MAXIMUM_DISK_CACHE_SIZE) {
            navigation.disk_cache_settings.maximum_size = PLAFOND_CACHE_HTTP;
            reglages.set_browsing_data_settings(navigation);
        }
        auto const& chemins = WebView::Application::profile().paths();
        warnln("[LB:CACHE] disque={} plafond_octets={} etiquette={} chemin={}",
            getenv("BOUCHAUD_DISABLE_DISK_CACHE") ? "non"sv : "oui"sv,
            reglages.browsing_data_settings().disk_cache_settings.maximum_size,
            cache_etiquete ? 1 : 0, chemins.cache);
        warnln("[LB:PROFILE] config={} donnees={} runtime={}", chemins.config, chemins.data, chemins.runtime);

        auto const& urls = WebView::Application::browser_options().urls;
        auto initiale = urls.is_empty() ? URL::about_blank() : urls.first();
        TRY(application->ouvre_la_fenetre(initiale, Web::DevicePixelSize { largeur, hauteur }));
        warnln("BOUCHAUD_UI_V1_READY onglets={}", application->fenetre()->nombre_de_vues());

        auto resultat = application->execute();
        if (resultat.is_error()) {
            warnln("[ladybird-bouchaud] BROWSER_HOST_EXIT erreur {}", resultat.error());
            return resultat.release_error();
        }
        code = resultat.value();
        warnln("[ladybird-bouchaud] BROWSER_HOST_EXIT boucle_quittee code={}", code);
    }
    warnln("[ladybird-bouchaud] BROWSER_HOST_ARRET services fermes");
    return code;
}
