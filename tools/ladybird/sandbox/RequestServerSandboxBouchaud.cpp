/*
 * Bouchaud OS -- bac a sable de RequestServer.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 *
 * BOUCHAUD_SANDBOX_V1 : RequestServer est le SEUL role qui possede le reseau
 * et le profil persistant (`BrowserNetwork`). Il reste confine : pas de
 * `no_new_privs` leve, aucune ecriture hors de son profil. Voir
 * `BouchaudConfinement.h`.
 */

#include "Sandbox.h"
#include <Services/BouchaudConfinement.h>

namespace RequestServer {

ErrorOr<void> apply_sandbox(Vector<ByteString> const&, StringView)
{
    return BouchaudConfinement::verifie(BouchaudConfinement::Role::Reseau, "RequestServer"sv);
}

}
