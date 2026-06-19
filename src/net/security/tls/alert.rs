//! Alertes TLS 1.3 (RFC 8446 §6) : niveaux et descriptions lisibles.
//!
//! Sans ce decodage, un echec serveur s'affichait en code brut
//! (`description=40`). On traduit ici les codes en noms standard
//! (`handshake_failure`, `unknown_ca`, `certificate_expired`...) pour rendre
//! les erreurs TLS comprehensibles dans la banniere et la trace `tls`.

/// Niveau d'alerte : 1 = warning, 2 = fatal.
pub fn level_name(level: u8) -> &'static str {
    match level {
        1 => "warning",
        2 => "fatal",
        _ => "inconnu",
    }
}

/// Description lisible d'un code d'alerte TLS (RFC 8446 §6, AlertDescription).
pub fn description(code: u8) -> &'static str {
    match code {
        0 => "close_notify",
        10 => "unexpected_message",
        20 => "bad_record_mac",
        21 => "decryption_failed",
        22 => "record_overflow",
        40 => "handshake_failure",
        41 => "no_certificate",
        42 => "bad_certificate",
        43 => "unsupported_certificate",
        44 => "certificate_revoked",
        45 => "certificate_expired",
        46 => "certificate_unknown",
        47 => "illegal_parameter",
        48 => "unknown_ca",
        49 => "access_denied",
        50 => "decode_error",
        51 => "decrypt_error",
        70 => "protocol_version",
        71 => "insufficient_security",
        80 => "internal_error",
        86 => "inappropriate_fallback",
        90 => "user_canceled",
        109 => "missing_extension",
        110 => "unsupported_extension",
        111 => "unsupported_certificate",
        112 => "unrecognized_name",
        113 => "bad_certificate_status_response",
        115 => "unknown_psk_identity",
        116 => "certificate_required",
        120 => "no_application_protocol",
        _ => "alerte inconnue",
    }
}

/// Message d'erreur statique pour une alerte recue pendant le handshake.
/// Donne le contexte le plus utile au diagnostic (cote serveur) pour les codes
/// courants, avec repli generique pour les autres.
pub fn handshake_error(code: u8) -> &'static str {
    match code {
        40 => "alerte TLS handshake_failure : le serveur a refuse la negociation (suites/groupes proposes)",
        47 => "alerte TLS illegal_parameter : parametre du ClientHello rejete par le serveur",
        48 => "alerte TLS unknown_ca : le serveur a rejete l'autorite (cote serveur)",
        50 => "alerte TLS decode_error : le serveur n'a pas su decoder notre message",
        51 => "alerte TLS decrypt_error : echec de verification cryptographique cote serveur",
        70 => "alerte TLS protocol_version : le serveur ne supporte pas TLS 1.3",
        71 => "alerte TLS insufficient_security : suites proposees jugees trop faibles",
        80 => "alerte TLS internal_error : erreur interne du serveur",
        109 => "alerte TLS missing_extension : extension obligatoire absente du ClientHello",
        110 => "alerte TLS unsupported_extension : extension non supportee par le serveur",
        112 => "alerte TLS unrecognized_name : SNI inconnu du serveur",
        116 => "alerte TLS certificate_required : le serveur exige un certificat client",
        120 => "alerte TLS no_application_protocol : aucun protocole ALPN commun",
        _ => "alerte TLS fatale pendant le handshake",
    }
}

/// Auto-test : verifie quelques mappings cles (RFC 8446 §6).
pub fn selftest() -> Result<(), &'static str> {
    if description(0) != "close_notify" { return Err("close_notify"); }
    if description(40) != "handshake_failure" { return Err("handshake_failure"); }
    if description(48) != "unknown_ca" { return Err("unknown_ca"); }
    if description(45) != "certificate_expired" { return Err("certificate_expired"); }
    if description(120) != "no_application_protocol" { return Err("no_application_protocol"); }
    if description(200) != "alerte inconnue" { return Err("repli inconnu"); }
    if level_name(2) != "fatal" { return Err("niveau fatal"); }
    if level_name(1) != "warning" { return Err("niveau warning"); }
    Ok(())
}
