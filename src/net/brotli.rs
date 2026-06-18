//! Brotli (RFC 7932) : decodage partiel cote client.
//!
//! Etat : en-tete de flux (WBITS), boucle de meta-blocs, meta-blocs
//! **non compresses** (ISUNCOMPRESSED) et metadata (ignores) entierement
//! geres. Les meta-blocs **compresses** renvoient `None` (repli propre vers
//! gzip/deflate) : un decodeur compresse complet exige aussi le dictionnaire
//! statique de 122 Ko (RFC 7932 Annexe A), un blob binaire fixe a embarquer et
//! a valider en environnement de test. Tant qu'il n'est pas present, on
//! n'annonce pas `br` dans `Accept-Encoding` (cf. net::http), donc les serveurs
//! repondent en gzip/deflate.
//!
//! Lecture des bits : LSB en premier dans chaque octet (convention Brotli).

use alloc::vec::Vec;

struct BitReader<'a> {
    data: &'a [u8],
    incnt: usize,
    bitbuf: u32,
    bitcnt: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader { data, incnt: 0, bitbuf: 0, bitcnt: 0 }
    }

    // Lit `need` bits (0..=24), LSB en premier.
    fn bits(&mut self, need: u32) -> Result<u32, ()> {
        if need == 0 { return Ok(0); }
        while self.bitcnt < need {
            if self.incnt >= self.data.len() { return Err(()); }
            self.bitbuf |= (self.data[self.incnt] as u32) << self.bitcnt;
            self.incnt += 1;
            self.bitcnt += 8;
        }
        let val = self.bitbuf & ((1u32 << need) - 1);
        self.bitbuf >>= need;
        self.bitcnt -= need;
        Ok(val)
    }

    // Aligne sur la frontiere d'octet suivante (rejette les bits partiels).
    // Apres alignement, `incnt` pointe sur le prochain octet a lire.
    fn align(&mut self) {
        // On a deja consomme `bitcnt` bits d'avance depuis `incnt` ; pour
        // revenir a la frontiere d'octet, on recule `incnt` du nombre d'octets
        // entiers encore tamponnes puis on jette les bits restants.
        let whole = (self.bitcnt / 8) as usize;
        self.incnt -= whole;
        self.bitbuf = 0;
        self.bitcnt = 0;
    }

    // Lit `n` octets bruts (apres `align`).
    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], ()> {
        if self.incnt + n > self.data.len() { return Err(()); }
        let s = &self.data[self.incnt..self.incnt + n];
        self.incnt += n;
        Ok(s)
    }
}

// Lit le champ WBITS (RFC 7932 §9.1) ; valeur ignoree ici (on n'a pas de fenetre
// glissante a borner pour le chemin non compresse), mais les bits doivent etre
// consommes pour rester aligne.
fn read_window_bits(br: &mut BitReader) -> Result<u32, ()> {
    if br.bits(1)? == 0 {
        return Ok(16);
    }
    let n = br.bits(3)?;
    if n != 0 {
        return Ok(17 + n);
    }
    let m = br.bits(3)?;
    if m != 0 {
        Ok(8 + m)
    } else {
        Ok(17)
    }
}

/// Decode un flux Brotli. Renvoie `None` si un meta-bloc compresse est
/// rencontre (non encore supporte) ou en cas de flux invalide.
pub fn decode(data: &[u8]) -> Option<Vec<u8>> {
    let mut br = BitReader::new(data);
    let mut out: Vec<u8> = Vec::new();
    read_window_bits(&mut br).ok()?;

    loop {
        let is_last = br.bits(1).ok()?;
        if is_last == 1 {
            let is_last_empty = br.bits(1).ok()?;
            if is_last_empty == 1 {
                return Some(out);
            }
        }
        let nibbles_code = br.bits(2).ok()?;
        if nibbles_code == 3 {
            // Meta-bloc de metadata : 1 bit reserve, MSKIPBYTES (2 bits),
            // MSKIPLEN, puis octets ignores apres alignement.
            let reserved = br.bits(1).ok()?;
            if reserved != 0 { return None; }
            let skip_bytes = br.bits(2).ok()?;
            let mut skip_len: usize = 0;
            if skip_bytes > 0 {
                for i in 0..skip_bytes {
                    let b = br.bits(8).ok()?;
                    skip_len |= (b as usize) << (8 * i);
                }
                skip_len += 1;
            }
            br.align();
            if skip_len > 0 {
                br.read_bytes(skip_len).ok()?;
            }
            if is_last == 1 {
                return Some(out);
            }
            continue;
        }

        let mnibbles = nibbles_code + 4; // 0->4, 1->5, 2->6
        let mut mlen_m1: usize = 0;
        for i in 0..mnibbles {
            let nib = br.bits(4).ok()?;
            mlen_m1 |= (nib as usize) << (4 * i);
        }
        let mlen = mlen_m1 + 1;

        // ISUNCOMPRESSED n'est present que pour les meta-blocs non terminaux.
        if is_last == 0 {
            let uncompressed = br.bits(1).ok()?;
            if uncompressed == 1 {
                br.align();
                let bytes = br.read_bytes(mlen).ok()?;
                out.extend_from_slice(bytes);
                continue;
            }
        }

        // Meta-bloc compresse : non encore supporte (necessite les codes de
        // prefixe + le dictionnaire statique). Repli propre.
        return None;
    }
}

/// Auto-test : flux Brotli compose d'un meta-bloc non compresse "abc" suivi du
/// meta-bloc final vide. Construit a la main d'apres RFC 7932 §9.
pub fn selftest() -> Result<(), &'static str> {
    // [WBITS=0][ISLAST=0][MNIBBLES=00->4][MLEN-1=2 sur 16 bits][ISUNCOMPRESSED=1]
    // -> alignement -> 'a''b''c' -> [ISLAST=1][ISLASTEMPTY=1].
    let stream = [0x20, 0x00, 0x10, b'a', b'b', b'c', 0x03];
    match decode(&stream) {
        Some(v) if v == b"abc" => Ok(()),
        Some(_) => Err("brotli uncompressed != abc"),
        None => Err("brotli uncompressed: None"),
    }
}
