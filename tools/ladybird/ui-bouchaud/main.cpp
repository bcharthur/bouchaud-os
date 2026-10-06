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
#include <LibMain/Main.h>
#include <LibURL/URL.h>
#include <UI/Bouchaud/Application.h>

#include <cstdlib>
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
    bool const ephemere = getenv("BOUCHAUD_LADYBIRD_EPHEMERAL") != nullptr;
    char const* home = ephemere ? "/tmp/ladybird" : "/persist/ladybird";
    char const* profil = ephemere ? "/tmp/ladybird-profile" : "/persist/ladybird/profile";
    char const* config = ephemere ? "/tmp/ladybird-config" : "/persist/ladybird/config";
    char const* donnees = ephemere ? "/tmp/ladybird-data" : "/persist/ladybird/data";
    char const* cache = ephemere ? "/tmp/ladybird-cache" : "/persist/ladybird/cache";
    char const* telechargements = ephemere ? "/tmp" : "/persist/Downloads";
    if (!ephemere)
        mkdir("/persist/ladybird", 0700);
    for (auto* dossier : { home, profil, config, donnees, cache })
        mkdir(dossier, 0700);
    if (!ephemere)
        mkdir("/persist/Downloads", 0755);

    setenv("HOME", home, 1);
    setenv("XDG_CONFIG_HOME", config, 1);
    setenv("XDG_RUNTIME_DIR", "/tmp/ladybird-runtime", 1);
    setenv("XDG_DATA_HOME", donnees, 1);
    setenv("XDG_CACHE_HOME", cache, 1);
    setenv("XDG_DOWNLOAD_DIR", telechargements, 1);

    if (getenv("BOUCHAUD_DISABLE_AUDIO") == nullptr) {
        setenv("SDL_AUDIODRIVER", "oss", 1);
        setenv("AUDIODEV", "/dev/dsp", 1);
    }

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
    arguments.append("--profile-path");
    arguments.append(profil);
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
    warnln("[LB:UI] profil persistant={} chemin={} telechargements={} ressources=/usr/share/ladybird",
        ephemere ? "non"sv : "oui"sv, profil, telechargements);
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
