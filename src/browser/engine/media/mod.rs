//! Médias temporels (vidéo / audio) pour le navigateur — un module par famille.
//!
//! Bouchaud OS n'a pas (encore) de pile de décodage A/V temps réel. Ce module
//! fournit l'**identification des conteneurs/codecs** (`probe`) pour que le moteur
//! HTML affiche un substitut cohérent (poster, contrôles, libellé du format) au
//! lieu d'un trou. Le décodage réel viendra par étapes (WAV PCM d'abord, le plus
//! simple, puis les conteneurs MP4/WebM).
//!
//!   - `video` : MP4 (ftyp), WebM/Matroska (EBML), Ogg ;
//!   - `audio` : WAV (PCM), MP3 (frame sync / ID3), Ogg, FLAC, AAC/M4A.

pub mod video;
pub mod audio;

/// Famille de média identifiée à partir des octets de tête.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MediaKind {
    Video(video::VideoFormat),
    Audio(audio::AudioFormat),
    Unknown,
}

/// Sonde le type de média d'après son en-tête (magic bytes).
pub fn probe(data: &[u8]) -> MediaKind {
    if let Some(v) = video::probe(data) { return MediaKind::Video(v); }
    if let Some(a) = audio::probe(data) { return MediaKind::Audio(a); }
    MediaKind::Unknown
}

/// Libellé court d'un type MIME média pour l'affichage du substitut.
pub fn label_for_mime(mime: &str) -> &'static str {
    let m = mime.trim();
    if m.starts_with("video/mp4") { "Vidéo MP4" }
    else if m.starts_with("video/webm") { "Vidéo WebM" }
    else if m.starts_with("video/ogg") { "Vidéo Ogg" }
    else if m.starts_with("video/") { "Vidéo" }
    else if m.starts_with("audio/mpeg") { "Audio MP3" }
    else if m.starts_with("audio/wav") || m.starts_with("audio/x-wav") { "Audio WAV" }
    else if m.starts_with("audio/ogg") { "Audio Ogg" }
    else if m.starts_with("audio/flac") { "Audio FLAC" }
    else if m.starts_with("audio/") { "Audio" }
    else { "Média" }
}
