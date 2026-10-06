/*
 * Bouchaud OS -- bac a sable des processus de rendu (WebContent, WebWorker).
 *
 * SPDX-License-Identifier: BSD-2-Clause
 *
 * BOUCHAUD_SANDBOX_V1 : remplace `RendererSandboxUnimplemented.cpp`, qui ne
 * faisait rien. Le confinement est applique par le noyau (profil
 * `BrowserContent`) ; ce processus le VERIFIE avant d'executer le moindre
 * script, et s'arrete s'il ne l'est pas. Voir `BouchaudConfinement.h`.
 */

#include <LibCore/System.h>
#include <Services/BouchaudConfinement.h>
#include <Services/RendererSandbox.h>

namespace RendererSandbox {

ErrorOr<void> apply_sandbox(Optional<StringView>, Optional<StringView>)
{
    auto chemin = TRY(Core::System::current_executable_path());
    auto service = chemin.view().find_last_split_view('/');
    return BouchaudConfinement::verifie(BouchaudConfinement::Role::Rendu, service);
}

}
