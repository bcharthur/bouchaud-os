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
// ## Pourquoi deux processus et non deux fils
//
// Parce que c'est la question du portage. Deux fils partagent un espace
// d'adressage : le socketpair y fonctionnerait meme si `fork` etait defaillant.
// Deux processus exercent ce que WebContent exigera — un `fork`, deux tables de
// descripteurs, et des messages qui traversent reellement le noyau.
//
// ## Les types
//
// `String`, `Vector<u32>`, `Optional<String>` et `URL::URL` sont les quatre
// codecs que LibIPC declare (`Encoder.h`, `Decoder.h`) et que LibWeb emploiera.
// Le fichier `.ipc` les nomme ; le generateur produit l'encodage ; nous n'avons
// ecrit aucun codec.

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
}

// Les valeurs de reference, partagees par les deux cotes. Ce sont elles que le
// serveur verifie a l'arrivee, et que le client verifie au retour : un codec
// qui perdrait un champ en chemin se verrait des deux cotes.
constexpr u32 VALEUR_PING = 42;
constexpr u32 VALEUR_PONG = 43;
StringView const TEXTE = "Bouchaud"sv;
StringView const URL_TEXTE = "https://bouchaud.example/portage?x=1#frag"sv;

Vector<u32> nombres_attendus()
{
    Vector<u32> v;
    v.append(1);
    v.append(1000);
    v.append(0xFFFFFFFFu); // le bit de poids fort : un codec qui signerait se trahirait
    return v;
}

// --- Le serveur -------------------------------------------------------------
//
// `ConnectionFromClient` fournit le proxy vers le client ; on herite du stub
// **genere** et on implemente ses methodes virtuelles.
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
        // On repond avec la valeur recue augmentee de un : si le codec perdait
        // la valeur, le client recevrait 1 et non 43.
        async_pong(valeur + 1);
    }

    virtual void echo(String texte, Vector<u32> nombres, Optional<String> option, URL::URL url) override
    {
        // Renvoi a l'identique : c'est l'aller-retour complet qui est mesure,
        // pas seulement l'aller.
        async_echo_result(texte, nombres, option, url);
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
        verifie("pong : la valeur a fait l'aller-retour", valeur == VALEUR_PONG);
        std::printf("         ping %u -> pong %u\n", VALEUR_PING, valeur);

        // Le second message part une fois le premier revenu : l'ordre rend le
        // diagnostic lisible si l'un des deux echoue.
        Vector<u32> nombres = nombres_attendus();
        async_echo(String::from_utf8(TEXTE).release_value(), nombres,
                   Optional<String> { String::from_utf8("presente"sv).release_value() },
                   URL::Parser::basic_parse(URL_TEXTE).release_value());
    }

    virtual void echo_result(String texte, Vector<u32> nombres, Optional<String> option, URL::URL url) override
    {
        verifie("String traverse intact", texte == TEXTE);

        auto attendus = nombres_attendus();
        bool vecteur_ok = nombres.size() == attendus.size();
        if (vecteur_ok) {
            for (size_t i = 0; i < nombres.size(); ++i)
                vecteur_ok = vecteur_ok && nombres[i] == attendus[i];
        }
        std::printf("         Vector<u32> : %zu elements, dernier = %u\n",
                    nombres.size(), nombres.is_empty() ? 0u : nombres.last());
        verifie("Vector<u32> traverse intact", vecteur_ok);

        verifie("Optional<String> renseigne traverse intact",
                option.has_value() && *option == "presente"sv);

        std::printf("         URL : %s\n", url.serialize().to_byte_string().characters());
        verifie("URL::URL traverse intacte", url.serialize() == URL_TEXTE);

        Core::EventLoop::current().quit(0);
    }
};

} // namespace

int main()
{
    std::printf("== temoin LibIPC : endpoints generes ==\n");

    // `socketpair` : la meme primitive que `IPC::Transport::create_pair()`
    // d'upstream emploie, et celle que Bouchaud fournit deja.
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
        auto transport = make<IPC::Transport>(MUST(Core::LocalSocket::adopt_fd(fds[1])));
        // `IPC::Connection` derive d'`EventReceiver`, donc de `RefCounted` :
        // une instance sur la pile part avec un compteur non nul et fait
        // echouer `!m_ref_count` a la destruction. Upstream passe par
        // `new_client_connection<T>` ; ici `make_ref_counted` suffit.
        auto serveur = make_ref_counted<Serveur>(move(transport));
        boucle.exec();
        _exit(0);
    }

    // --- Processus A : le client --------------------------------------------
    close(fds[1]);
    Core::EventLoop boucle;
    auto transport = make<IPC::Transport>(MUST(Core::LocalSocket::adopt_fd(fds[0])));
    auto client = make_ref_counted<Client>(move(transport));

    // Le premier message part du fil principal, avant la boucle : le proxy
    // genere se charge de l'encoder et de l'ecrire.
    client->async_ping(VALEUR_PING);

    // Un garde-fou : sans lui, un codec qui ne repondrait jamais laisserait le
    // temoin suspendu, et le harnais ne verrait qu'un delai depasse — ce qui ne
    // designe aucune cause.
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
