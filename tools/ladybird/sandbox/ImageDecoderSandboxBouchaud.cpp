/*
 * Bouchaud OS -- bac a sable d'ImageDecoder.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 *
 * BOUCHAUD_SANDBOX_V1 : ImageDecoder decode les octets d'images venues du
 * reseau ; c'est un role de RENDU (`BrowserContent`). Voir
 * `BouchaudConfinement.h`.
 */

#include "Sandbox.h"
#include <Services/BouchaudConfinement.h>

namespace ImageDecoder {

ErrorOr<void> apply_sandbox()
{
    return BouchaudConfinement::verifie(BouchaudConfinement::Role::Rendu, "ImageDecoder"sv);
}

}
