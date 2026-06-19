//! Couche application (OSI L7) : services et protocoles applicatifs.
//!   - resolution de noms (DNS) et configuration dynamique (DHCP) ;
//!   - HTTP/1.1 et HTTP/2 (+ HPACK) ;
//!   - analyse/rendu texte HTML.
pub mod dns;
pub mod dhcp;
pub mod http;
pub mod http2;
pub mod hpack;
pub mod html;
