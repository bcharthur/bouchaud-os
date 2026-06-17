//! Grands entiers non signes (pour RSA). Representation little-endian en limbes u32.
//!
//! Le strict necessaire pour la VERIFICATION RSA (exponentiation modulaire avec
//! exposant public court). Pas optimise (pas de Montgomery) : suffisant car la
//! verification n'effectue qu'une poignee d'operations.

use alloc::vec::Vec;

#[derive(Clone, PartialEq, Eq)]
pub struct BigUint {
    /// Limbes little-endian (limbs[0] = poids faible), sans zeros de tete.
    pub limbs: Vec<u32>,
}

impl BigUint {
    pub fn zero() -> Self { BigUint { limbs: Vec::new() } }

    pub fn from_bytes_be(bytes: &[u8]) -> Self {
        // Decoupe en limbes de 32 bits depuis l'octet de poids faible.
        let mut limbs = Vec::new();
        let mut i = bytes.len();
        while i > 0 {
            let start = if i >= 4 { i - 4 } else { 0 };
            let mut v = 0u32;
            for &b in &bytes[start..i] {
                v = (v << 8) | b as u32;
            }
            limbs.push(v);
            i = start;
        }
        let mut b = BigUint { limbs };
        b.normalize();
        b
    }

    pub fn to_bytes_be(&self) -> Vec<u8> {
        if self.limbs.is_empty() { return alloc::vec![0u8]; }
        let mut out = Vec::new();
        for &limb in self.limbs.iter().rev() {
            out.extend_from_slice(&limb.to_be_bytes());
        }
        // Retire les zeros de tete (mais garde au moins un octet).
        let mut start = 0;
        while start + 1 < out.len() && out[start] == 0 { start += 1; }
        out[start..].to_vec()
    }

    fn normalize(&mut self) {
        while let Some(&0) = self.limbs.last() {
            self.limbs.pop();
        }
    }

    pub fn is_zero(&self) -> bool { self.limbs.is_empty() }

    pub fn bit_len(&self) -> usize {
        match self.limbs.last() {
            None => 0,
            Some(&top) => (self.limbs.len() - 1) * 32 + (32 - top.leading_zeros() as usize),
        }
    }

    pub fn get_bit(&self, i: usize) -> u32 {
        let limb = i / 32;
        let off = i % 32;
        if limb >= self.limbs.len() { 0 } else { (self.limbs[limb] >> off) & 1 }
    }

    fn cmp(&self, other: &BigUint) -> core::cmp::Ordering {
        use core::cmp::Ordering;
        if self.limbs.len() != other.limbs.len() {
            return self.limbs.len().cmp(&other.limbs.len());
        }
        for i in (0..self.limbs.len()).rev() {
            if self.limbs[i] != other.limbs[i] {
                return self.limbs[i].cmp(&other.limbs[i]);
            }
        }
        Ordering::Equal
    }

    // self >= other ?
    fn ge(&self, other: &BigUint) -> bool {
        self.cmp(other) != core::cmp::Ordering::Less
    }

    // self -= other (suppose self >= other)
    fn sub_assign(&mut self, other: &BigUint) {
        let mut borrow = 0i64;
        for i in 0..self.limbs.len() {
            let o = if i < other.limbs.len() { other.limbs[i] as i64 } else { 0 };
            let mut cur = self.limbs[i] as i64 - o - borrow;
            if cur < 0 { cur += 1 << 32; borrow = 1; } else { borrow = 0; }
            self.limbs[i] = cur as u32;
        }
        self.normalize();
    }

    // Decalage a gauche d'un bit, avec injection d'un bit de poids faible.
    fn shl1_or(&mut self, bit: u32) {
        let mut carry = bit & 1;
        for limb in self.limbs.iter_mut() {
            let new_carry = *limb >> 31;
            *limb = (*limb << 1) | carry;
            carry = new_carry;
        }
        if carry != 0 { self.limbs.push(carry); }
    }

    /// Addition.
    pub fn add(&self, other: &BigUint) -> BigUint {
        let n = self.limbs.len().max(other.limbs.len());
        let mut out = Vec::with_capacity(n + 1);
        let mut carry = 0u64;
        for i in 0..n {
            let a = *self.limbs.get(i).unwrap_or(&0) as u64;
            let b = *other.limbs.get(i).unwrap_or(&0) as u64;
            let cur = a + b + carry;
            out.push(cur as u32);
            carry = cur >> 32;
        }
        if carry != 0 { out.push(carry as u32); }
        let mut r = BigUint { limbs: out };
        r.normalize();
        r
    }

    /// Soustraction publique (suppose self >= other).
    pub fn sub(&self, other: &BigUint) -> BigUint {
        let mut r = self.clone();
        r.sub_assign(other);
        r
    }

    /// Comparaison publique.
    pub fn cmp_pub(&self, other: &BigUint) -> core::cmp::Ordering {
        self.cmp(other)
    }

    /// Multiplication scolaire.
    pub fn mul(&self, other: &BigUint) -> BigUint {
        if self.is_zero() || other.is_zero() { return BigUint::zero(); }
        let mut out = alloc::vec![0u32; self.limbs.len() + other.limbs.len()];
        for i in 0..self.limbs.len() {
            let mut carry = 0u64;
            let a = self.limbs[i] as u64;
            for j in 0..other.limbs.len() {
                let cur = out[i + j] as u64 + a * other.limbs[j] as u64 + carry;
                out[i + j] = cur as u32;
                carry = cur >> 32;
            }
            let mut k = i + other.limbs.len();
            while carry != 0 {
                let cur = out[k] as u64 + carry;
                out[k] = cur as u32;
                carry = cur >> 32;
                k += 1;
            }
        }
        let mut b = BigUint { limbs: out };
        b.normalize();
        b
    }

    /// Reduction modulaire self mod m (division binaire bit a bit).
    pub fn rem(&self, m: &BigUint) -> BigUint {
        if m.is_zero() { return BigUint::zero(); }
        let mut r = BigUint::zero();
        for i in (0..self.bit_len()).rev() {
            r.shl1_or(self.get_bit(i));
            if r.ge(m) {
                r.sub_assign(m);
            }
        }
        r
    }

    /// Exponentiation modulaire : self^exp mod m (square-and-multiply).
    pub fn modpow(&self, exp: &BigUint, m: &BigUint) -> BigUint {
        if m.is_zero() { return BigUint::zero(); }
        let mut result = BigUint::from_bytes_be(&[1]);
        result = result.rem(m);
        let base = self.rem(m);
        let bits = exp.bit_len();
        for i in (0..bits).rev() {
            result = result.mul(&result).rem(m);
            if exp.get_bit(i) == 1 {
                result = result.mul(&base).rem(m);
            }
        }
        result
    }
}
