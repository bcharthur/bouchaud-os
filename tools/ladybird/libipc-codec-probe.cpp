// Temoin M5a/M5b : les endpoints **generes** de LibIPC, entre deux processus.
//
// Les tests d'upstream (`Tests/LibIPC/`) prouvent deja le transport : le
// raccrochage du pair, la fin de fichier, le passage de descripteurs, la
// concurrence. Ils ne passent pas par le generateur : ils construisent leurs
// messages a la main.
//
// Ce temoin verifie l'autre moitie, celle que le transport ne peut pas
// prouver :
//
//     BouchaudPortage*.ipc
//          v
//     Meta/Generators/generate_ipc_definitions.py   (le generateur d'upstream)
//          v
//     Proxy / Stub / Endpoint generes
//          v
//     IPC::ConnectionFromClient / ConnectionToServer
//          v
//     socketpair Bouchaud
//          v
//     aller-retour reel
//
// Rien n'est encode ni decode a la main ici. On appelle `async_ping(42)` sur un
// proxy genere, et on attend que la methode virtuelle du stub genere soit
// appelee a l'autre bout avec 42.
//
// ## Un type a la fois, puis tous ensemble
//
// Les messages sont envoyes **en chaine** : chaque reponse declenche l'envoi
// suivant. `String`, puis `Vector<u32>`, puis `Optional<String>`, puis
// `URL::URL`, et enfin les quatre reunis.
//
// Ce n'est pas de la prudence : c'est ce qui rend un echec diagnostiquable. Un
// message unique portant quatre types dit seulement « quelque chose n'est pas
// revenu ». La chaine dit **lequel**, et la trace en dit le sens — l'aller ou
// le retour.
//
// ## Pourquoi deux processus et non deux fils
//
// Parce que c'est la question du portage. Deux fils partagent un espace
// d'adressage : le socketpair y fonctionnerait meme si `fork` etait defaillant.
// Deux processus exercent ce que WebContent exigera — un `fork`, deux tables de
// descripteurs, et des messages qui traversent reellement le noyau.

#include <BouchaudPortageClientEndpoint.h>
#include <BouchaudPortageServerEndpoint.h>

#include <AK/String.h>
#include <LibCore/EventLoop.h>
#include <LibCore/Timer.h>
#include <LibIPC/ConnectionFromClient.h>
#include <LibIPC/ConnectionToServer.h>
#include <LibIPC/Transport.h>
#include <LibURL/Parser.h>

#include <cstdio>
#include <sys/socket.h>
#include <sys/wait.h>
#include <unistd.h>

namespace {

int echecs = 0;
int passees = 0;

void verifie(char const* quoi, bool ok)
{
    if (ok) {
        ++passees;
        std::printf("  ok     %s\n", quoi);
    } else {
        ++echecs;
        std::printf("  ECHEC  %s\n", quoi);
    }
    std::fflush(stdout);
}

// Trace de progression. Elle nomme le **dernier point atteint** : quand un
// message ne revient pas, c'est la derniere ligne imprimee qui designe l'etape
// fautive, et de quel cote elle se trouve.
void trace(char const* cote, char const* etape)
{
    std::printf("         [%s] %s\n", cote, etape);
    std::fflush(stdout);
}

constexpr u32 VALEUR_PING = 42;
constexpr u32 VALEUR_PONG = 43;
StringView const TEXTE = "Bouchaud"sv;
StringView const OPTION = "presente"sv;
StringView const URL_TEXTE = "https://example.com/path?q=42#fragment"sv;

Vector<u32> nombres_attendus()
{
    Vector<u32> v;
    v.append(1);
    v.append(1000);
    v.append(0xFFFFFFFFu); // le bit de poids fort : un codec qui signerait se trahirait
    return v;
}

String texte() { return String::from_utf8(TEXTE).release_value(); }
String option() { return String::from_utf8(OPTION).release_value(); }
URL::URL adresse() { return URL::Parser::basic_parse(URL_TEXTE).release_value(); }

// --- Le serveur -------------------------------------------------------------
class Serveur final : public IPC::ConnectionFromClient<BouchaudPortageClientEndpoint, BouchaudPortageServerEndpoint> {
public:
    explicit Serveur(NonnullOwnPtr<IPC::Transport> transport)
        : IPC::ConnectionFromClient<BouchaudPortageClientEndpoint, BouchaudPortageServerEndpoint>(*this, move(transport), 1)
    {
    }

    virtual void die() override { Core::EventLoop::current().quit(0); }

private:
    virtual void ping(u32 valeur) override
    {
        trace("serveur", "ping recu");
        async_pong(valeur + 1);
    }

    virtual void echo_string(String recu) override
    {
        trace("serveur", "echo_string recu");
        async_echo_string_result(recu);
    }

    virtual void echo_vector(Vector<u32> nombres) override
    {
        trace("serveur", "echo_vector recu");
        async_echo_vector_result(nombres);
    }

    virtual void echo_optional(Optional<String> opt) override
    {
        trace("serveur", "echo_optional recu");
        async_echo_optional_result(opt);
    }

    virtual void echo_url(URL::URL url) override
    {
        trace("serveur", "echo_url recu");
        async_echo_url_result(url);
    }

    virtual void echo(String recu, Vector<u32> nombres, Optional<String> opt, URL::URL url) override
    {
        trace("serveur", "echo (4 champs) recu");
        async_echo_result(recu, nombres, opt, url);
    }
};

// --- Le client --------------------------------------------------------------
class Client final : public IPC::ConnectionToServer<BouchaudPortageClientEndpoint, BouchaudPortageServerEndpoint> {
public:
    explicit Client(NonnullOwnPtr<IPC::Transport> transport)
        : IPC::ConnectionToServer<BouchaudPortageClientEndpoint, BouchaudPortageServerEndpoint>(*this, move(transport))
    {
    }

    virtual void die() override { Core::EventLoop::current().quit(1); }

private:
    virtual void pong(u32 valeur) override
    {
        verifie("u32 : ping 42 -> pong 43", valeur == VALEUR_PONG);
        trace("client", "envoi echo_string");
        async_echo_string(texte());
    }

    virtual void echo_string_result(String recu) override
    {
        verifie("String traverse intact", recu == TEXTE);
        trace("client", "envoi echo_vector");
        async_echo_vector(nombres_attendus());
    }

    virtual void echo_vector_result(Vector<u32> nombres) override
    {
        auto attendus = nombres_attendus();
        bool ok = nombres.size() == attendus.size();
        for (size_t i = 0; ok && i < nombres.size(); ++i)
            ok = nombres[i] == attendus[i];
        std::printf("         Vector<u32> : %zu elements, dernier = %u\n",
                    nombres.size(), nombres.is_empty() ? 0u : nombres.last());
        verifie("Vector<u32> traverse intact", ok);
        trace("client", "envoi echo_optional");
        async_echo_optional(Optional<String> { option() });
    }

    virtual void echo_optional_result(Optional<String> recu) override
    {
        verifie("Optional<String> renseigne traverse intact",
                recu.has_value() && *recu == OPTION);
        trace("client", "envoi echo_url");
        async_echo_url(adresse());
    }

    virtual void echo_url_result(URL::URL recue) override
    {
        std::printf("         URL : %s\n", recue.serialize().to_byte_string().characters());
        verifie("URL::URL traverse intacte", recue.serialize() == URL_TEXTE);
        trace("client", "envoi echo (4 champs)");
        async_echo(texte(), nombres_attendus(), Optional<String> { option() }, adresse());
    }

    virtual void echo_result(String recu, Vector<u32> nombres, Optional<String> opt, URL::URL url) override
    {
        auto attendus = nombres_attendus();
        bool ok = recu == TEXTE
            && nombres.size() == attendus.size()
            && opt.has_value() && *opt == OPTION
            && url.serialize() == URL_TEXTE;
        verifie("les quatre types dans un seul message", ok);
        Core::EventLoop::current().quit(0);
    }
};

} // namespace

int main()
{
    std::printf("== temoin LibIPC : endpoints generes ==\n");

    int fds[2] {};
    if (socketpair(AF_LOCAL, SOCK_STREAM, 0, fds) < 0) {
        std::printf("  ECHEC  socketpair : %s\n", strerror(errno));
        std::printf("\nRESULTAT : 1 verification(s) en echec\n");
        return 1;
    }

    auto pid = fork();
    if (pid < 0) {
        std::printf("  ECHEC  fork : %s\n", strerror(errno));
        std::printf("\nRESULTAT : 1 verification(s) en echec\n");
        return 1;
    }

    if (pid == 0) {
        // --- Processus B : le serveur ---------------------------------------
        close(fds[0]);
        Core::EventLoop boucle;
        // `IPC::Connection` derive d'`EventReceiver`, donc de `RefCounted` :
        // une instance sur la pile part avec un compteur non nul et fait
        // echouer `!m_ref_count` a la destruction.
        auto serveur = make_ref_counted<Serveur>(make<IPC::Transport>(MUST(Core::LocalSocket::adopt_fd(fds[1]))));
        boucle.exec();
        _exit(0);
    }

    // --- Processus A : le client --------------------------------------------
    close(fds[1]);
    Core::EventLoop boucle;
    auto client = make_ref_counted<Client>(make<IPC::Transport>(MUST(Core::LocalSocket::adopt_fd(fds[0]))));

    trace("client", "envoi ping");
    client->async_ping(VALEUR_PING);

    // Un garde-fou : sans lui, un message qui ne revient pas laisserait le
    // temoin suspendu, et le harnais ne verrait qu'un delai depasse — ce qui ne
    // designe aucune cause. La trace, elle, nomme le dernier point atteint.
    auto minuteur = Core::Timer::create_single_shot(10000, [&] {
        std::printf("  ECHEC  aucune reponse en 10 s\n");
        ++echecs;
        Core::EventLoop::current().quit(1);
    });
    minuteur->start();

    boucle.exec();

    client->shutdown();
    int etat = 0;
    waitpid(pid, &etat, 0);

    std::printf("\nRESULTAT : %d verification(s) en echec (%d passees)\n", echecs, passees);
    return echecs == 0 ? 0 : 1;
}
