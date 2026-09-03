use super::capability::Capabilities;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecurityProfile {
    System,
    User,
    BrowserBroker,
    /// Les roles de RENDU : WebContent, WebWorker, ImageDecoder.
    ///
    /// Ils analysent des donnees venues du reseau. Ce sont eux qu'un site
    /// hostile atteint en premier, et ce sont donc eux qui doivent avoir le
    /// moins d'autorite : ni reseau, ni exec, ni peripherique.
    BrowserContent,
    /// Le SEUL role de navigateur qui possede le reseau : RequestServer.
    ///
    /// BOUCHAUD_C6_REQUESTSERVER_N_EST_PAS_UN_RENDU_V1
    ///
    /// Il etait classe `BrowserContent`, exactement comme WebContent. Les deux
    /// avaient donc la meme autorite, et l'architecture ou « le reseau
    /// appartient a RequestServer » ne pouvait pas etre appliquee : lui retirer
    /// le reseau l'aurait casse, le lui laisser l'aurait donne au moteur de
    /// rendu. Les separer est ce qui rend la regle exprimable.
    BrowserNetwork,
    Untrusted,
}

fn untrusted_path(image: &str) -> bool {
    image.starts_with("/tmp/")
        || image.starts_with("/var/tmp/")
        || image.contains("/Downloads/")
}

fn browser_content_image(image: &str) -> bool {
    image.ends_with("/WebContent")
        || image.ends_with("/WebWorker")
        || image.ends_with("/ImageDecoder")
}

fn browser_network_image(image: &str) -> bool {
    image.ends_with("/RequestServer")
}

fn trusted_browser_broker_image(image: &str) -> bool {
    image == "/usr/bin/bo-navigateur"
        || image.ends_with("/usr/bin/BrowserHost")
        || image.ends_with("/usr/bin/WebDriver")
        || image.ends_with("/usr/bin/Compositor")
        || (image.starts_with("/usr/libexec/ladybird/")
            && (image.ends_with("/BrowserHost")
                || image.ends_with("/WebDriver")
                || image.ends_with("/Compositor")))
}

pub fn classify(image: &str, uid: u32) -> SecurityProfile {
    // Location wins over name. A binary copied into /tmp cannot become a broker
    // merely by calling itself BrowserHost.
    if untrusted_path(image) {
        return SecurityProfile::Untrusted;
    }

    if browser_content_image(image) {
        return SecurityProfile::BrowserContent;
    }

    if browser_network_image(image) {
        return SecurityProfile::BrowserNetwork;
    }

    if trusted_browser_broker_image(image) {
        return SecurityProfile::BrowserBroker;
    }

    if uid == 0 {
        SecurityProfile::System
    } else {
        SecurityProfile::User
    }
}

pub const fn capabilities(profile: SecurityProfile) -> Capabilities {
    match profile {
        SecurityProfile::System => Capabilities::ALL,
        SecurityProfile::User => Capabilities(
            Capabilities::EXEC.0
                | Capabilities::IPC_TRANSFER.0
                | Capabilities::NET_CONNECT.0
        ),
        SecurityProfile::BrowserBroker => Capabilities(
            Capabilities::EXEC.0
                | Capabilities::JIT.0
                | Capabilities::DEVICE_IO.0
                | Capabilities::IPC_TRANSFER.0
                | Capabilities::NET_CONNECT.0
        ),
        // Le rendu garde EXEC comme AUTORITE D'IMAGE -- c'est ce que le
        // courtier consulte pour avoir le droit de LANCER un WebContent. Ce
        // qu'il n'a pas, c'est le droit d'exec en tant qu'APPELANT :
        // `execution::appelant_peut_executer` le refuse, et c'est la que se
        // joue la difference entre « ce binaire peut etre lance » et « ce
        // processus peut lancer ».
        SecurityProfile::BrowserContent => Capabilities(
            Capabilities::EXEC.0
                | Capabilities::JIT.0
                | Capabilities::IPC_TRANSFER.0
        ),
        // RequestServer : le reseau, et rien d'autre. Pas de JIT -- il
        // n'execute aucun script --, pas de peripherique.
        SecurityProfile::BrowserNetwork => Capabilities(
            Capabilities::EXEC.0
                | Capabilities::IPC_TRANSFER.0
                | Capabilities::NET_CONNECT.0
        ),
        SecurityProfile::Untrusted => Capabilities::EXEC,
    }
}

/// Authority available to a brand-new process from its login identity.  A
/// non-root process can therefore never gain DEVICE_IO by choosing a special
/// executable name/path; a root-launched trusted broker can deliberately drop
/// from ALL to the broker set.
pub fn initial_capabilities(image: &str, uid: u32) -> Capabilities {
    let ambient = if uid == 0 {
        Capabilities::ALL
    } else {
        capabilities(SecurityProfile::User)
    };
    ambient.intersection(capabilities(classify(image, uid)))
}

/// Every ordinary security-domain transition is monotonic.  Exec, identity
/// changes and sandboxing may remove authority but never manufacture a bit that
/// the process did not already own.
pub const fn transition_capabilities(
    current: Capabilities,
    wanted: SecurityProfile,
) -> Capabilities {
    current.intersection(capabilities(wanted))
}

/// Ce profil est-il SANDBOXE ?
///
/// Un role sandboxe traite des donnees venues du reseau, ou vient d'un
/// emplacement qui n'est pas de confiance. Il porte `no_new_privs` d'office,
/// n'a pas le droit d'exec en tant qu'appelant, et son acces au systeme de
/// fichiers est restreint a des chemins nommes.
///
/// Le rendre explicite evite que chaque controle reconstruise sa propre liste
/// -- et qu'un profil ajoute plus tard soit oublie dans l'une d'elles.
pub const fn sandboxe(profile: SecurityProfile) -> bool {
    matches!(
        profile,
        SecurityProfile::BrowserContent
            | SecurityProfile::BrowserNetwork
            | SecurityProfile::Untrusted
    )
}
