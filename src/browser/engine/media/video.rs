//! Identification des conteneurs vidéo. Le décodage temporel n'est pas encore
//! disponible (codecs H.264/VP8/VP9/AV1 hors budget actuel) ; `probe` permet au
//! moteur HTML de rendre un substitut adapté pour `<video>`.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VideoFormat {
    Mp4,
    WebM,
    Ogg,
}

/// Identifie le conteneur vidéo d'après l'en-tête.
pub fn probe(data: &[u8]) -> Option<VideoFormat> {
    if data.len() >= 12 && &data[4..8] == b"ftyp" {
        // ISO-BMFF (MP4/MOV/M4V). brand dans data[8..12].
        return Some(VideoFormat::Mp4);
    }
    if data.len() >= 4 && data[0] == 0x1A && data[1] == 0x45 && data[2] == 0xDF && data[3] == 0xA3 {
        // EBML -> Matroska/WebM.
        return Some(VideoFormat::WebM);
    }
    if data.len() >= 4 && &data[..4] == b"OggS" {
        return Some(VideoFormat::Ogg);
    }
    None
}
