#!/usr/bin/env python3
'''
Bouchaud platform completion layer for the disposable pinned Ladybird worktree.

Runs AFTER prepare-full-browser-host.py.

This layer deliberately reuses WebView::Application's upstream implementations:
clipboard state, downloads, profile stores, ProcessManager/WorkerProcessManager,
Compositor and HeadlessWebView child-view/fullscreen plumbing.
'''
from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-platform-complete.py <ladybird-worktree>")

root = Path(sys.argv[1])
host = root / "Services/BouchaudBrowserHost/main.cpp"
if not host.is_file():
    raise SystemExit("platform-complete: BouchaudBrowserHost/main.cpp absent")

data = host.read_text()

def replace_once(old: str, new: str, label: str) -> None:
    global data
    if new in data:
        return
    if old not in data:
        raise SystemExit(f"platform-complete: ancre introuvable ({label})")
    data = data.replace(old, new, 1)

old = r'''    mkdir("/tmp", 0777);
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
'''
new = r'''    mkdir("/tmp", 0777);
    mkdir("/tmp/ladybird-runtime", 0700);
    mkdir("/tmp/fontconfig", 0777);

    // `/persist` est monte par Bouchaud avant l'autorun.
    mkdir("/persist/ladybird", 0777);
    mkdir("/persist/ladybird/profile", 0777);
    mkdir("/persist/ladybird/config", 0777);
    mkdir("/persist/ladybird/data", 0777);
    mkdir("/persist/ladybird/cache", 0777);
    mkdir("/persist/Downloads", 0777);

    chmod("/tmp/ladybird-runtime", 0700);

    bool ephemeral_profile = getenv("BOUCHAUD_LADYBIRD_EPHEMERAL") != nullptr;
    char const* home = ephemeral_profile ? "/tmp/ladybird" : "/persist/ladybird";
    char const* profile = ephemeral_profile ? "/tmp/ladybird-profile" : "/persist/ladybird/profile";

    if (ephemeral_profile) {
        mkdir("/tmp/ladybird", 0777);
        mkdir("/tmp/ladybird-profile", 0777);
        mkdir("/tmp/ladybird-config", 0777);
        mkdir("/tmp/ladybird-data", 0777);
        mkdir("/tmp/ladybird-cache", 0777);
    }

    setenv("HOME", home, 1);
    setenv("XDG_CONFIG_HOME", ephemeral_profile ? "/tmp/ladybird-config" : "/persist/ladybird/config", 1);
    setenv("XDG_RUNTIME_DIR", "/tmp/ladybird-runtime", 1);
    setenv("XDG_DATA_HOME", ephemeral_profile ? "/tmp/ladybird-data" : "/persist/ladybird/data", 1);
    setenv("XDG_CACHE_HOME", ephemeral_profile ? "/tmp/ladybird-cache" : "/persist/ladybird/cache", 1);
    setenv("XDG_DOWNLOAD_DIR", ephemeral_profile ? "/tmp" : "/persist/Downloads", 1);

    if (getenv("BOUCHAUD_DISABLE_AUDIO") == nullptr) {
        setenv("SDL_AUDIODRIVER", "oss", 1);
        setenv("AUDIODEV", "/dev/dsp", 1);
    }
'''
replace_once(old, new, "persistent profile/XDG")

old = r'''    arguments.append("--force-cpu-painting");
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
'''
new = r'''    arguments.append("--force-cpu-painting");
    arguments.append("--force-fontconfig");

    // Le sandbox Linux n'est pas un sandbox Bouchaud.
    arguments.append("--disable-sandbox");

    // Restaurer les comportements upstream par defaut; les desactivations
    // deviennent uniquement des drapeaux de diagnostic.
    if (getenv("BOUCHAUD_DISABLE_DISK_CACHE"))
        arguments.append("--disable-http-disk-cache");
    if (getenv("BOUCHAUD_DISABLE_SQL"))
        arguments.append("--disable-sql-database");
    if (getenv("BOUCHAUD_DISABLE_ASYNC_SCROLLING"))
        arguments.append("--disable-async-scrolling");

    char const* site_isolation = getenv("BOUCHAUD_SITE_ISOLATION");
    if (!site_isolation || !*site_isolation)
        site_isolation = "top-level";
    arguments.append("--site-isolation");
    arguments.append(site_isolation);

    if (getenv("BOUCHAUD_ALLOW_POPUPS"))
        arguments.append("--allow-popups");
    if (getenv("BOUCHAUD_ENABLE_AUTOPLAY"))
        arguments.append("--enable-autoplay");

    char const* timezone = getenv("BOUCHAUD_TIME_ZONE");
    if (!timezone || !*timezone)
        timezone = "Europe/Paris";
    arguments.append("--default-time-zone");
    arguments.append(timezone);

    arguments.append("--profile-path");
    arguments.append(profile);
'''
replace_once(old, new, "restore upstream feature defaults")

old = r'''    outln("[ladybird-bouchaud] BROWSER_HOST_PATHS runtime=/tmp/ladybird-runtime data=/tmp/ladybird-data cache=/tmp/ladybird-cache resources=/usr/share/ladybird");
    outln("[ladybird-bouchaud] BROWSER_HOST_STORAGE sql=disabled upstream-jars=in-memory");
    outln("[ladybird-bouchaud] BROWSER_HOST_SERVICES RequestServer ImageDecoder Compositor WebContent WebWorker:on-demand");'''
new = r'''    outln("[ladybird-bouchaud] BROWSER_HOST_PATHS runtime=/tmp/ladybird-runtime profile={} downloads={} resources=/usr/share/ladybird",
        profile, ephemeral_profile ? "/tmp" : "/persist/Downloads");
    outln("[ladybird-bouchaud] BROWSER_HOST_PLATFORM persistent={} sql={} disk_cache={} async_scrolling={} site_isolation={} timezone={} audio={}",
        ephemeral_profile ? "no" : "yes",
        getenv("BOUCHAUD_DISABLE_SQL") ? "disabled" : "enabled",
        getenv("BOUCHAUD_DISABLE_DISK_CACHE") ? "disabled" : "enabled",
        getenv("BOUCHAUD_DISABLE_ASYNC_SCROLLING") ? "disabled" : "enabled",
        site_isolation,
        timezone,
        getenv("BOUCHAUD_DISABLE_AUDIO") ? "disabled" : "/dev/dsp");
    outln("[ladybird-bouchaud] BROWSER_HOST_UPSTREAM clipboard=text downloads=FileDownloader popups=HeadlessWebView fullscreen=HeadlessWebView");
    outln("[ladybird-bouchaud] BROWSER_HOST_SERVICES RequestServer ImageDecoder Compositor WebContent WebWorker:on-demand WebDriver=packaged");'''
replace_once(old, new, "platform diagnostics")

host.write_text(data)

print("Ladybird platform completion applied")
