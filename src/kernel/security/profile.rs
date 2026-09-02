use super::capability::Capabilities;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecurityProfile {
    System,
    User,
    BrowserBroker,
    BrowserContent,
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
        || image.ends_with("/RequestServer")
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
        ),
        SecurityProfile::BrowserBroker => Capabilities(
            Capabilities::EXEC.0
                | Capabilities::JIT.0
                | Capabilities::DEVICE_IO.0
                | Capabilities::IPC_TRANSFER.0
        ),
        SecurityProfile::BrowserContent => Capabilities(
            Capabilities::EXEC.0
                | Capabilities::JIT.0
                | Capabilities::IPC_TRANSFER.0
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
