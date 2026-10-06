/*
 * Bouchaud OS -- le bac a sable des services Ladybird, cote processus.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 *
 * BOUCHAUD_SANDBOX_V1
 *
 * Sous Linux, un service Ladybird se confine LUI-MEME (`SandboxLinux.cpp` :
 * no_new_privs, Landlock, seccomp). Sous Bouchaud, c'est le NOYAU qui confine,
 * des l'exec, selon le profil que l'image recoit
 * (`src/kernel/security/profile.rs`) : `BrowserContent` pour les roles de rendu
 * (WebContent, WebWorker, ImageDecoder, Compositor), `BrowserNetwork` pour
 * RequestServer. Le processus n'a rien a demander.
 *
 * Il a en revanche quelque chose a VERIFIER. Un rendu qui tournerait sans
 * confinement -- binaire renomme, table de profils qui derive, image lancee
 * depuis un chemin inattendu -- executerait le script des sites avec les
 * droits de l'utilisateur. Ce controle le detecte AVANT la premiere ligne de
 * script, et refuse de continuer (fail-closed) :
 *
 *   - `no_new_privs` est pose (le noyau le pose pour tout role sandboxe) ;
 *   - ecrire dans `/usr` est refuse (aucun role sandboxe ne le peut ; un
 *     processus non confine lance par root le pourrait) ;
 *   - pour un rendu : ecrire dans le profil (`/persist/ladybird`), le depot de
 *     telechargements et le magasin du chrome est refuse, et ouvrir une socket
 *     est refuse (pas de `NET_CONNECT`).
 *
 * Une sonde qui REUSSIT est un echec du bac a sable : le fichier cree est
 * aussitot retire, et l'erreur remonte jusqu'a `main`, qui s'arrete.
 * `ENOENT` sur un dossier qui n'existe pas (profil ephemere) n'est pas une
 * reussite : rien n'a ete cree, et le noyau aurait refuse de toute facon.
 */

#pragma once

#include <AK/ByteString.h>
#include <AK/Error.h>
#include <AK/Format.h>
#include <AK/StringView.h>

#include <cerrno>
#include <fcntl.h>
#include <sys/prctl.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <unistd.h>

namespace BouchaudConfinement {

enum class Role {
    Rendu,
    Reseau,
};

/// Rend vrai si l'ecriture a ete REFUSEE (ou si la cible n'existe pas).
inline bool ecriture_refusee(StringView dossier, StringView service, int& errno_vu)
{
    auto chemin = ByteString::formatted("{}/.bouchaud-confinement-{}-{}", dossier, service, getpid());
    int fd = open(chemin.characters(), O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0600);
    if (fd >= 0) {
        close(fd);
        unlink(chemin.characters());
        errno_vu = 0;
        return false;
    }
    // Tout echec de creation veut dire que rien n'a ete ecrit : seule une
    // REUSSITE est une breche. L'errno est garde pour le journal.
    errno_vu = errno;
    return true;
}

inline ErrorOr<void> verifie(Role role, StringView service)
{
    auto const nnp = prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0);
    if (nnp != 1) {
        warnln("[LB:SANDBOX] ECHEC service={} no_new_privs={} -- le noyau n'a pas confine ce processus", service, nnp);
        return Error::from_string_literal("Bouchaud: no_new_privs absent, processus non confine");
    }

    int errno_usr = 0;
    if (!ecriture_refusee("/usr"sv, service, errno_usr)) {
        warnln("[LB:SANDBOX] ECHEC service={} ecriture /usr acceptee errno={}", service, errno_usr);
        return Error::from_string_literal("Bouchaud: ecriture /usr acceptee, processus non confine");
    }

    if (role == Role::Reseau) {
        warnln("[LB:SANDBOX] service={} role=reseau no_new_privs=1 ecriture_usr=refusee(errno={})", service, errno_usr);
        return {};
    }

    for (auto dossier : { "/persist/ladybird"sv, "/persist/Downloads"sv, "/persist/ladybird-chrome"sv }) {
        int errno_vu = 0;
        if (!ecriture_refusee(dossier, service, errno_vu)) {
            warnln("[LB:SANDBOX] ECHEC service={} ecriture {} acceptee errno={}", service, dossier, errno_vu);
            return Error::from_string_literal("Bouchaud: un rendu peut ecrire le stockage persistant");
        }
    }

    int s = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (s >= 0) {
        close(s);
        warnln("[LB:SANDBOX] ECHEC service={} socket AF_INET acceptee", service);
        return Error::from_string_literal("Bouchaud: un rendu peut ouvrir une socket reseau");
    }
    auto const errno_socket = errno;

    warnln("[LB:SANDBOX] service={} role=rendu no_new_privs=1 ecriture_usr=refusee(errno={}) persistant=refuse reseau=refuse(errno={})",
        service, errno_usr, errno_socket);
    return {};
}

}
