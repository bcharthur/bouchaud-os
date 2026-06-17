//! X25519 (RFC 7748) — echange de cles Diffie-Hellman sur Curve25519.
//!
//! Portage fidele de l'implementation de reference 64 bits "curve25519-donna"
//! (domaine public, D. J. Bernstein / Adam Langley), representation radix 2^51.

type Felem = [u64; 5];
const MASK: u64 = 0x7ffffffffffff; // 2^51 - 1

fn fsum(output: &mut Felem, input: &Felem) {
    for i in 0..5 { output[i] += input[i]; }
}

// output = input - output  (attention a l'ordre des arguments)
fn fdifference_backwards(out: &mut Felem, input: &Felem) {
    let two54m152: u64 = (1u64 << 54) - 152;
    let two54m8: u64 = (1u64 << 54) - 8;
    out[0] = input[0] + two54m152 - out[0];
    out[1] = input[1] + two54m8 - out[1];
    out[2] = input[2] + two54m8 - out[2];
    out[3] = input[3] + two54m8 - out[3];
    out[4] = input[4] + two54m8 - out[4];
}

fn fscalar_product(output: &mut Felem, input: &Felem, scalar: u64) {
    let mut a: u128 = (input[0] as u128) * scalar as u128;
    output[0] = (a as u64) & MASK;
    a = (input[1] as u128) * scalar as u128 + ((a >> 51) as u64) as u128;
    output[1] = (a as u64) & MASK;
    a = (input[2] as u128) * scalar as u128 + ((a >> 51) as u64) as u128;
    output[2] = (a as u64) & MASK;
    a = (input[3] as u128) * scalar as u128 + ((a >> 51) as u64) as u128;
    output[3] = (a as u64) & MASK;
    a = (input[4] as u128) * scalar as u128 + ((a >> 51) as u64) as u128;
    output[4] = (a as u64) & MASK;
    output[0] += (a >> 51) as u64 * 19;
}

fn fmul(output: &mut Felem, in2: &Felem, input: &Felem) {
    let mut t = [0u128; 5];
    let (r0, r1, r2, r3, r4) = (input[0], input[1], input[2], input[3], input[4]);
    let (s0, s1, s2, s3, s4) = (in2[0], in2[1], in2[2], in2[3], in2[4]);

    t[0] = (r0 as u128) * s0 as u128;
    t[1] = (r0 as u128) * s1 as u128 + (r1 as u128) * s0 as u128;
    t[2] = (r0 as u128) * s2 as u128 + (r2 as u128) * s0 as u128 + (r1 as u128) * s1 as u128;
    t[3] = (r0 as u128) * s3 as u128 + (r3 as u128) * s0 as u128 + (r1 as u128) * s2 as u128 + (r2 as u128) * s1 as u128;
    t[4] = (r0 as u128) * s4 as u128 + (r4 as u128) * s0 as u128 + (r3 as u128) * s1 as u128 + (r1 as u128) * s3 as u128 + (r2 as u128) * s2 as u128;

    let r4m = r4 * 19;
    let r1m = r1 * 19;
    let r2m = r2 * 19;
    let r3m = r3 * 19;

    t[0] += (r4m as u128) * s1 as u128 + (r1m as u128) * s4 as u128 + (r2m as u128) * s3 as u128 + (r3m as u128) * s2 as u128;
    t[1] += (r4m as u128) * s2 as u128 + (r2m as u128) * s4 as u128 + (r3m as u128) * s3 as u128;
    t[2] += (r4m as u128) * s3 as u128 + (r3m as u128) * s4 as u128;
    t[3] += (r4m as u128) * s4 as u128;

    let mut c: u64;
    let mut rr = [0u64; 5];
    rr[0] = (t[0] as u64) & MASK; c = (t[0] >> 51) as u64;
    t[1] += c as u128; rr[1] = (t[1] as u64) & MASK; c = (t[1] >> 51) as u64;
    t[2] += c as u128; rr[2] = (t[2] as u64) & MASK; c = (t[2] >> 51) as u64;
    t[3] += c as u128; rr[3] = (t[3] as u64) & MASK; c = (t[3] >> 51) as u64;
    t[4] += c as u128; rr[4] = (t[4] as u64) & MASK; c = (t[4] >> 51) as u64;
    rr[0] += c * 19; c = rr[0] >> 51; rr[0] &= MASK;
    rr[1] += c; c = rr[1] >> 51; rr[1] &= MASK;
    rr[2] += c;
    *output = rr;
}

fn fsquare_times(output: &mut Felem, input: &Felem, count: u64) {
    let (mut r0, mut r1, mut r2, mut r3, mut r4) =
        (input[0], input[1], input[2], input[3], input[4]);
    let mut n = count;
    loop {
        let d0 = r0 * 2;
        let d1 = r1 * 2;
        let d2 = r2 * 2 * 19;
        let d419 = r4 * 19;
        let d4 = d419 * 2;

        let mut t = [0u128; 5];
        t[0] = (r0 as u128) * r0 as u128 + (d4 as u128) * r1 as u128 + (d2 as u128) * r3 as u128;
        t[1] = (d0 as u128) * r1 as u128 + (d4 as u128) * r2 as u128 + (r3 as u128) * (r3 * 19) as u128;
        t[2] = (d0 as u128) * r2 as u128 + (r1 as u128) * r1 as u128 + (d4 as u128) * r3 as u128;
        t[3] = (d0 as u128) * r3 as u128 + (d1 as u128) * r2 as u128 + (r4 as u128) * d419 as u128;
        t[4] = (d0 as u128) * r4 as u128 + (d1 as u128) * r3 as u128 + (r2 as u128) * r2 as u128;

        let mut c: u64;
        r0 = (t[0] as u64) & MASK; c = (t[0] >> 51) as u64;
        t[1] += c as u128; r1 = (t[1] as u64) & MASK; c = (t[1] >> 51) as u64;
        t[2] += c as u128; r2 = (t[2] as u64) & MASK; c = (t[2] >> 51) as u64;
        t[3] += c as u128; r3 = (t[3] as u64) & MASK; c = (t[3] >> 51) as u64;
        t[4] += c as u128; r4 = (t[4] as u64) & MASK; c = (t[4] >> 51) as u64;
        r0 += c * 19; c = r0 >> 51; r0 &= MASK;
        r1 += c; c = r1 >> 51; r1 &= MASK;
        r2 += c;

        n -= 1;
        if n == 0 { break; }
    }
    *output = [r0, r1, r2, r3, r4];
}

fn fexpand(input: &[u8; 32]) -> Felem {
    let rd = |off: usize| -> u64 {
        let mut b = [0u8; 8];
        b.copy_from_slice(&input[off..off + 8]);
        u64::from_le_bytes(b)
    };
    let mut o = [0u64; 5];
    o[0] = rd(0) & MASK;
    o[1] = (rd(6) >> 3) & MASK;
    o[2] = (rd(12) >> 6) & MASK;
    o[3] = (rd(19) >> 1) & MASK;
    o[4] = (rd(24) >> 12) & MASK;
    o
}

fn fcontract(input: &Felem) -> [u8; 32] {
    let mut t = [0u128; 5];
    for i in 0..5 { t[i] = input[i] as u128; }

    for _ in 0..2 {
        t[1] += t[0] >> 51; t[0] &= MASK as u128;
        t[2] += t[1] >> 51; t[1] &= MASK as u128;
        t[3] += t[2] >> 51; t[2] &= MASK as u128;
        t[4] += t[3] >> 51; t[3] &= MASK as u128;
        t[0] += 19 * (t[4] >> 51); t[4] &= MASK as u128;
    }

    t[0] += 19;
    t[1] += t[0] >> 51; t[0] &= MASK as u128;
    t[2] += t[1] >> 51; t[1] &= MASK as u128;
    t[3] += t[2] >> 51; t[2] &= MASK as u128;
    t[4] += t[3] >> 51; t[3] &= MASK as u128;
    t[0] += 19 * (t[4] >> 51); t[4] &= MASK as u128;

    t[0] += 0x8000000000000 - 19;
    t[1] += 0x8000000000000 - 1;
    t[2] += 0x8000000000000 - 1;
    t[3] += 0x8000000000000 - 1;
    t[4] += 0x8000000000000 - 1;

    t[1] += t[0] >> 51; t[0] &= MASK as u128;
    t[2] += t[1] >> 51; t[1] &= MASK as u128;
    t[3] += t[2] >> 51; t[2] &= MASK as u128;
    t[4] += t[3] >> 51; t[3] &= MASK as u128;
    t[4] &= MASK as u128;

    let f0 = (t[0] as u64) | ((t[1] as u64) << 51);
    let f1 = ((t[1] as u64) >> 13) | ((t[2] as u64) << 38);
    let f2 = ((t[2] as u64) >> 26) | ((t[3] as u64) << 25);
    let f3 = ((t[3] as u64) >> 39) | ((t[4] as u64) << 12);

    let mut out = [0u8; 32];
    out[0..8].copy_from_slice(&f0.to_le_bytes());
    out[8..16].copy_from_slice(&f1.to_le_bytes());
    out[16..24].copy_from_slice(&f2.to_le_bytes());
    out[24..32].copy_from_slice(&f3.to_le_bytes());
    out
}

fn swap_conditional(a: &mut Felem, b: &mut Felem, iswap: u64) {
    let swap = 0u64.wrapping_sub(iswap);
    for i in 0..5 {
        let x = swap & (a[i] ^ b[i]);
        a[i] ^= x;
        b[i] ^= x;
    }
}

#[allow(clippy::too_many_arguments)]
fn fmonty(
    x2: &mut Felem, z2: &mut Felem,
    x3: &mut Felem, z3: &mut Felem,
    x: &mut Felem, z: &mut Felem,
    xprime: &mut Felem, zprime: &mut Felem,
    qmqp: &Felem,
) {
    let origx = *x;
    fsum(x, z);
    fdifference_backwards(z, &origx);

    let origxprime = *xprime;
    fsum(xprime, zprime);
    fdifference_backwards(zprime, &origxprime);
    let mut xxprime = [0u64; 5];
    let mut zzprime = [0u64; 5];
    fmul(&mut xxprime, xprime, z);
    fmul(&mut zzprime, x, zprime);
    let origxprime2 = xxprime;
    fsum(&mut xxprime, &zzprime);
    fdifference_backwards(&mut zzprime, &origxprime2);
    fsquare_times(x3, &xxprime, 1);
    let mut zzzprime = [0u64; 5];
    fsquare_times(&mut zzzprime, &zzprime, 1);
    fmul(z3, &zzzprime, qmqp);

    let mut xx = [0u64; 5];
    let mut zz = [0u64; 5];
    fsquare_times(&mut xx, x, 1);
    fsquare_times(&mut zz, z, 1);
    fmul(x2, &xx, &zz);
    fdifference_backwards(&mut zz, &xx);
    let mut zzz = [0u64; 5];
    fscalar_product(&mut zzz, &zz, 121665);
    fsum(&mut zzz, &xx);
    fmul(z2, &zz, &zzz);
}

fn cmult(resultx: &mut Felem, resultz: &mut Felem, n: &[u8; 32], q: &Felem) {
    let mut nqpqx: Felem = *q;
    let mut nqpqz: Felem = [1, 0, 0, 0, 0];
    let mut nqx: Felem = [1, 0, 0, 0, 0];
    let mut nqz: Felem = [0, 0, 0, 0, 0];

    let mut nqpqx2: Felem = [0; 5];
    let mut nqpqz2: Felem = [1, 0, 0, 0, 0];
    let mut nqx2: Felem = [0; 5];
    let mut nqz2: Felem = [1, 0, 0, 0, 0];

    for i in 0..32 {
        let mut byte = n[31 - i];
        for _ in 0..8 {
            let bit = (byte >> 7) as u64;

            swap_conditional(&mut nqx, &mut nqpqx, bit);
            swap_conditional(&mut nqz, &mut nqpqz, bit);
            fmonty(
                &mut nqx2, &mut nqz2,
                &mut nqpqx2, &mut nqpqz2,
                &mut nqx, &mut nqz,
                &mut nqpqx, &mut nqpqz,
                q,
            );
            swap_conditional(&mut nqx2, &mut nqpqx2, bit);
            swap_conditional(&mut nqz2, &mut nqpqz2, bit);

            core::mem::swap(&mut nqx, &mut nqx2);
            core::mem::swap(&mut nqz, &mut nqz2);
            core::mem::swap(&mut nqpqx, &mut nqpqx2);
            core::mem::swap(&mut nqpqz, &mut nqpqz2);

            byte <<= 1;
        }
    }
    *resultx = nqx;
    *resultz = nqz;
}

fn crecip(z: &Felem) -> Felem {
    let mut a = [0u64; 5];
    let mut t0 = [0u64; 5];
    let mut b = [0u64; 5];
    let mut c = [0u64; 5];
    fsquare_times(&mut a, z, 1);          // 2
    fsquare_times(&mut t0, &a, 2);        // 8
    fmul(&mut b, &t0, z);                 // 9
    let a_copy = a; fmul(&mut a, &b, &a_copy); // 11
    fsquare_times(&mut t0, &a, 1);        // 22
    let b_copy = b; fmul(&mut b, &t0, &b_copy); // 2^5-2^0
    fsquare_times(&mut t0, &b, 5);
    let b_copy = b; fmul(&mut b, &t0, &b_copy);
    fsquare_times(&mut t0, &b, 10);
    fmul(&mut c, &t0, &b);
    fsquare_times(&mut t0, &c, 20);
    let t0_copy = t0; fmul(&mut t0, &t0_copy, &c);
    let t0_copy = t0; fsquare_times(&mut t0, &t0_copy, 10);
    let b_copy = b; fmul(&mut b, &t0, &b_copy);
    fsquare_times(&mut t0, &b, 50);
    fmul(&mut c, &t0, &b);
    fsquare_times(&mut t0, &c, 100);
    let t0_copy = t0; fmul(&mut t0, &t0_copy, &c);
    let t0_copy = t0; fsquare_times(&mut t0, &t0_copy, 50);
    let t0_copy = t0; fmul(&mut t0, &t0_copy, &b);
    let t0_copy = t0; fsquare_times(&mut t0, &t0_copy, 5);
    let mut out = [0u64; 5];
    fmul(&mut out, &t0, &a);
    out
}

/// Calcule scalaire * point (X25519). `secret` est clampe selon RFC 7748.
pub fn x25519(secret: &[u8; 32], basepoint: &[u8; 32]) -> [u8; 32] {
    let mut e = *secret;
    e[0] &= 248;
    e[31] &= 127;
    e[31] |= 64;
    let bp = fexpand(basepoint);
    let mut x = [0u64; 5];
    let mut z = [0u64; 5];
    cmult(&mut x, &mut z, &e, &bp);
    let zmone = crecip(&z);
    let mut out = [0u64; 5];
    fmul(&mut out, &x, &zmone);
    fcontract(&out)
}

/// Multiplie le scalaire par le point de base standard (9). Pour la cle publique.
pub fn base_mul(secret: &[u8; 32]) -> [u8; 32] {
    let mut base = [0u8; 32];
    base[0] = 9;
    x25519(secret, &base)
}

/// Auto-test : vecteurs RFC 7748 section 5.2.
pub fn selftest() -> Result<(), &'static str> {
    // Cas 1
    let scalar = hex32("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4");
    let point = hex32("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c");
    let out = x25519(&scalar, &point);
    let want = hex32("c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552");
    if out != want { return Err("x25519 rfc7748 #1"); }

    // base * base * ... : un tour d'iteration (apres 1 iter le resultat est connu).
    let mut k = [0u8; 32]; k[0] = 9;
    let mut u = k;
    let r = x25519(&k, &u);
    let want1 = hex32("422c8e7a6227d7bca1350b3e2bb7279f7897b87bb6854b783c60e80311ae3079");
    if r != want1 { return Err("x25519 iter1"); }
    let _ = &mut u;

    Ok(())
}

fn hex32(s: &str) -> [u8; 32] {
    let b = s.as_bytes();
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = (hexv(b[2 * i]) << 4) | hexv(b[2 * i + 1]);
        i += 1;
    }
    out
}

fn hexv(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}
