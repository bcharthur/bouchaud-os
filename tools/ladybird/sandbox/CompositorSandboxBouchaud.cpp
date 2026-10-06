/*
 * Bouchaud OS -- bac a sable du Compositor.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 *
 * BOUCHAUD_SANDBOX_V1 : le Compositor rejoue les listes d'affichage que
 * WebContent -- donc le script d'un site -- a produites. C'est un role de
 * RENDU (`BrowserContent`), pas un courtier : il n'a besoin ni du reseau, ni
 * des peripheriques, ni du stockage persistant. En peinture CPU il n'ecrit
 * aucun cache (le cache de `cache_path` est celui des shaders GPU). Voir
 * `BouchaudConfinement.h`.
 */

#include "Sandbox.h"
#include <Services/BouchaudConfinement.h>

namespace Compositor {

ErrorOr<void> apply_sandbox(StringView)
{
    return BouchaudConfinement::verifie(BouchaudConfinement::Role::Rendu, "Compositor"sv);
}

}
