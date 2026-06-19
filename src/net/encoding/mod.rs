//! Couche presentation (OSI L6) : codages de transfert de contenu.
//!   - `inflate` : DEFLATE / zlib / gzip ;
//!   - `brotli`  : decodeur Brotli (RFC 7932) + dictionnaire embarque
//!     (brotli_tables.rs et brotli_dict.bin sont inclus par brotli.rs).
pub mod inflate;
pub mod brotli;
