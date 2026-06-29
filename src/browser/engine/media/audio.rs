//! Identification des formats audio. Le WAV PCM est le candidat le plus simple
//! pour un futur décodage réel (lecture directe d'échantillons) ; `probe` couvre
//! déjà l'identification pour le substitut de `<audio>`.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AudioFormat {
    Wav,
    Mp3,
    Ogg,
    Flac,
    Aac,
}

/// Identifie le format audio d'après l'en-tête.
pub fn probe(data: &[u8]) -> Option<AudioFormat> {
    if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WAVE" {
        return Some(AudioFormat::Wav);
    }
    if data.len() >= 4 && &data[..4] == b"fLaC" {
        return Some(AudioFormat::Flac);
    }
    if data.len() >= 4 && &data[..4] == b"OggS" {
        return Some(AudioFormat::Ogg);
    }
    if data.len() >= 3 && &data[..3] == b"ID3" {
        return Some(AudioFormat::Mp3); // tag ID3 -> presque toujours MP3
    }
    if data.len() >= 2 && data[0] == 0xFF && (data[1] & 0xE0) == 0xE0 {
        // sync MPEG audio frame (MP3) ou ADTS AAC (0xFFF...).
        if (data[1] & 0xF6) == 0xF0 { return Some(AudioFormat::Aac); }
        return Some(AudioFormat::Mp3);
    }
    None
}

/// Métadonnées PCM d'un fichier WAV (pour un futur lecteur d'échantillons).
#[derive(Clone, Copy, Debug)]
pub struct WavInfo {
    pub channels: u16,
    pub sample_rate: u32,
    pub bits: u16,
    pub data_off: usize,
    pub data_len: usize,
}

/// Lit l'en-tête WAV (chunk `fmt ` + `data`) sans décoder les échantillons.
pub fn wav_info(d: &[u8]) -> Option<WavInfo> {
    if d.len() < 12 || &d[..4] != b"RIFF" || &d[8..12] != b"WAVE" { return None; }
    let mut p = 12usize;
    let mut fmt: Option<(u16, u32, u16)> = None;
    while p + 8 <= d.len() {
        let id = &d[p..p + 4];
        let sz = (d[p + 4] as usize) | ((d[p + 5] as usize) << 8) | ((d[p + 6] as usize) << 16) | ((d[p + 7] as usize) << 24);
        let body = p + 8;
        if id == b"fmt " && body + 16 <= d.len() {
            let channels = (d[body + 2] as u16) | ((d[body + 3] as u16) << 8);
            let rate = (d[body + 4] as u32) | ((d[body + 5] as u32) << 8) | ((d[body + 6] as u32) << 16) | ((d[body + 7] as u32) << 24);
            let bits = (d[body + 14] as u16) | ((d[body + 15] as u16) << 8);
            fmt = Some((channels, rate, bits));
        } else if id == b"data" {
            let (channels, sample_rate, bits) = fmt?;
            return Some(WavInfo { channels, sample_rate, bits, data_off: body, data_len: sz.min(d.len() - body) });
        }
        p = body + sz + (sz & 1); // chunks alignés sur 2 octets
    }
    None
}
