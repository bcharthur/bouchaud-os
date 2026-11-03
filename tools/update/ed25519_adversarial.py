#!/usr/bin/env python3
"""Adversarial Ed25519 encodings: Bouchaud vs OpenSSL vs libsodium
(BOUCHAUD_ED25519_ADVERSARIAL_V1).

    python3 tools/update/ed25519_adversarial.py ORACLE [--strict]

ORACLE is the binary built from tools/update/ed25519_oracle.rs (the
production verifier, unchanged). References, each optional but reported:
  * OpenSSL  through `cryptography` (cofactorless, RFC 8032 S < L check);
  * libsodium through PyNaCl (rejects small-order and non-canonical points).

Every pathological input is BUILT here with the RFC 8032 section 6 reference
arithmetic (no vector is copied from memory). Bouchaud's policy is the STRICT
one -- RFC 8032 decoding (y < p, no "negative zero"), S < L, and in addition
no small-order public key or R (libsodium's rule): a release key and the R
of an honest signature are never small order, so refusing them costs nothing
and closes every cofactor/identity trick.

The script prints one line per case and implementation, and with --strict
exits 1 if Bouchaud deviates from the expected verdict, or accepts something
libsodium rejects.
"""
from __future__ import annotations

import hashlib
import os
import subprocess
import sys

# ---- RFC 8032 section 6 reference arithmetic --------------------------------
P = 2**255 - 19
L = 2**252 + 27742317777372353535851937790883648493
D = -121665 * pow(121666, P - 2, P) % P
SQRT_M1 = pow(2, (P - 1) // 4, P)


def add(a, b):
    A = (a[1] - a[0]) * (b[1] - b[0]) % P
    B = (a[1] + a[0]) * (b[1] + b[0]) % P
    C = 2 * a[3] * b[3] * D % P
    Dd = 2 * a[2] * b[2] % P
    E, F, G, H = B - A, Dd - C, Dd + C, B + A
    return (E * F % P, G * H % P, F * G % P, E * H % P)


def mul(s, pt):
    q = (0, 1, 1, 0)
    while s > 0:
        if s & 1:
            q = add(q, pt)
        pt = add(pt, pt)
        s >>= 1
    return q


def eq(a, b):
    return (a[0] * b[2] - b[0] * a[2]) % P == 0 and (a[1] * b[2] - b[1] * a[2]) % P == 0


def recover_x(y, sign):
    if y >= P:
        return None
    x2 = (y * y - 1) * pow(D * y * y + 1, P - 2, P) % P
    if x2 == 0:
        return None if sign else 0
    x = pow(x2, (P + 3) // 8, P)
    if (x * x - x2) % P != 0:
        x = x * SQRT_M1 % P
    if (x * x - x2) % P != 0:
        return None
    if (x & 1) != sign:
        x = P - x
    return x


G_Y = 4 * pow(5, P - 2, P) % P
G_X = recover_x(G_Y, 0)
G = (G_X, G_Y, 1, G_X * G_Y % P)
IDENT = (0, 1, 1, 0)


def compress(pt):
    zi = pow(pt[2], P - 2, P)
    x, y = pt[0] * zi % P, pt[1] * zi % P
    return int.to_bytes(y | ((x & 1) << 255), 32, "little")


def decompress(b):
    y = int.from_bytes(b, "little")
    sign = y >> 255
    y &= (1 << 255) - 1
    x = recover_x(y, sign)
    if x is None:
        return None
    return (x, y, 1, x * y % P)


def raw(y, sign):
    return int.to_bytes(y | (sign << 255), 32, "little")


def h(*parts):
    return int.from_bytes(hashlib.sha512(b"".join(parts)).digest(), "little")


def keypair(seed):
    hd = hashlib.sha512(seed).digest()
    a = int.from_bytes(hd[:32], "little")
    a &= (1 << 254) - 8
    a |= 1 << 254
    return a, hd[32:], compress(mul(a, G))


def sign(a, prefix, pk, msg):
    r = h(prefix, msg) % L
    R = compress(mul(r, G))
    k = h(R, pk, msg) % L
    S = (r + k * a) % L
    return R + int.to_bytes(S, 32, "little")


# ---- the 8-torsion subgroup, computed -----------------------------------
def torsion():
    pts = []
    y = 2
    while len(pts) < 8:
        q = decompress(int.to_bytes(y, 32, "little"))
        y += 1
        if q is None:
            continue
        t = mul(L, q)          # kills the prime-order part
        order = next((n for n in (1, 2, 4, 8) if eq(mul(n, t), IDENT)), None)
        if order == 8:
            pts = [mul(i, t) for i in range(8)]
    return pts


# ---- implementations -------------------------------------------------------
def bouchaud(oracle, cases):
    data = "".join(f"{pk.hex()} {(m.hex() or '-')} {s.hex()}\n" for _, pk, m, s, _ in cases)
    out = subprocess.run([oracle], input=data, capture_output=True, text=True, check=True).stdout.split()
    return [o == "1" for o in out]


def openssl(cases):
    try:
        from cryptography.exceptions import InvalidSignature
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
    except ImportError:
        return None
    res = []
    for _, pk, m, s, _ in cases:
        try:
            Ed25519PublicKey.from_public_bytes(pk).verify(s, m)
            res.append(True)
        except (InvalidSignature, ValueError):
            res.append(False)
    return res


def libsodium(cases):
    try:
        import nacl.bindings as b
        from nacl.exceptions import BadSignatureError
    except ImportError:
        return None
    res = []
    for _, pk, m, s, _ in cases:
        try:
            b.crypto_sign_open(s + m, pk)
            res.append(True)
        except (BadSignatureError, ValueError, Exception):
            res.append(False)
    return res


def main():
    oracle = sys.argv[1]
    strict = "--strict" in sys.argv
    T = torsion()
    a, prefix, pk = keypair(b"bouchaud-adversarial-seed-0001".ljust(32, b"\0"))
    msg = b"bouchaud-manifeste-v1\nversion=x\n"
    good = sign(a, prefix, pk, msg)
    R, S = good[:32], int.from_bytes(good[32:], "little")
    ident_enc = compress(IDENT)
    A = decompress(pk)
    y_pk = A[1] * pow(A[2], P - 2, P) % P
    cases = []  # (name, pk, msg, sig, expected_bouchaud)

    def c(name, pk_, sig_, exp, m=msg):
        cases.append((name, pk_, m, sig_, exp))

    c("valid", pk, good, True)
    # --- S range (RFC 8032 5.1.7: S in [0, L)) ---
    c("S=S+L (malleated)", pk, R + int.to_bytes(S + L, 32, "little"), False)
    c("S=L", pk, R + int.to_bytes(L, 32, "little"), False)
    c("S>L (2^253)", pk, R + int.to_bytes(2**253, 32, "little"), False)
    c("S=0 R=valid", pk, R + bytes(32), False)
    c("S=2^256-1", pk, R + b"\xff" * 32, False)
    # --- public key encodings ---
    for i, t in enumerate(T):
        e = compress(t)
        c(f"pk=torsion[{i}] R=identity S=0", e, ident_enc + bytes(32), False)
        c(f"pk=torsion[{i}] R=torsion[{i}] S=0", e, e + bytes(32), False)
    c("pk=identity R=identity S=0 (accepts any msg if not refused)", ident_enc, ident_enc + bytes(32), False)
    c("pk=identity R=identity S=0, other msg", ident_enc, ident_enc + bytes(32), False, m=b"anything")
    # non-canonical identity encodings: y = 1 + p (fits in 255 bits), negative zero
    # The real forgery under a small-order key: R = [r]B (full order), S = r.
    B_enc = compress(G)
    for i, t in enumerate(T):
        c(f"pk=torsion[{i}] R=B S=1 (forgery for any msg)", compress(t), B_enc + int.to_bytes(1, 32, "little"), False)
        c(f"pk=torsion[{i}] R=[7]B S=7, other msg", compress(t), compress(mul(7, G)) + int.to_bytes(7, 32, "little"), False, m=b"x")
    c("pk=identity non-canonical y=1+p", raw(1 + P, 0), ident_enc + bytes(32), False)
    c("pk=identity negative-zero (x=0,sign=1)", raw(1, 1), ident_enc + bytes(32), False)
    # non-canonical encoding of the REAL key, when y + p < 2^255
    if y_pk + P < 2**255:
        c("pk=real key, y+p (non-canonical)", raw(y_pk + P, compress(A)[31] >> 7), good, False)
    else:
        print("note  real key y >= 2^255 - p: no non-canonical y form exists for it")
    # pk not on the curve
    yy = 2
    while decompress(int.to_bytes(yy, 32, "little")) is not None:
        yy += 1
    c("pk=not a point", raw(yy, 0), good, False)
    c("pk=y=p (non-canonical 0)", raw(P, 0), good, False)
    c("pk=y=2^255-1", raw(2**255 - 1, 0), good, False)
    # --- R encodings ---
    c("R=identity S=valid", pk, ident_enc + good[32:], False)
    # The KEY HOLDER signing with nonce r = 0: R = identity, S = k*a. The
    # cofactorless equation HOLDS; only the small-order R rule refuses it
    # (libsodium refuses, OpenSSL accepts). Without this case, deleting the
    # R check goes unnoticed (mutation found 2026-11-03).
    for m_ in (msg, b"autre"):
        k0 = h(ident_enc, pk, m_) % L
        c(f"R=identity S=k*a (holder, r=0) msg={m_[:6]!r}", pk, ident_enc + int.to_bytes(k0 * a % L, 32, "little"), False, m=m_)
    for i, t in enumerate(T):
        c(f"R=torsion[{i}] (S=valid)", pk, compress(t) + good[32:], False)
    c("R=not a point", pk, raw(yy, 0) + good[32:], False)
    c("R=non-canonical y+p of R", pk, (raw((int.from_bytes(R, 'little') & ((1 << 255) - 1)) + P, R[31] >> 7)
                                         if (int.from_bytes(R, 'little') & ((1 << 255) - 1)) + P < 2**255 else b"\xff" * 32) + good[32:], False)
    # R + small-order component: same cofactored equation, cofactorless fails
    R_pt = decompress(R)
    c("R=R+torsion[1] (cofactored-only valid)", pk, compress(add(R_pt, T[1])) + good[32:], False)
    # A + small order: key mixed with torsion, signature made for it honestly
    # would need the secret; we check the verifier does not accept the
    # original signature under the mixed key.
    c("pk=A+torsion[1] (mixed order) with good sig", compress(add(A, T[1])), good, False)

    # --- y boundary encodings, as public keys and as R ---
    for nom, y, sg in [("y=0 sign=0 (order 4)", 0, 0), ("y=0 sign=1 (order 4)", 0, 1),
                       ("y=p-1 sign=0 (order 2)", P - 1, 0), ("y=p-1 sign=1 (negative zero)", P - 1, 1),
                       ("y=1 sign=0 (identity)", 1, 0), ("y=p-2", P - 2, 0), ("y=p+18 (max 255-bit)", P + 18, 0),
                       ("y=2^255-20 = p-1", 2**255 - 20, 0)]:
        e = raw(y, sg)
        c(f"pk={nom}", e, good, False)
        c(f"R={nom}", pk, e + good[32:], False)
    # --- random honest signatures, then single alterations ---
    rnd = __import__("random").Random(0xB0)
    for i in range(64):
        sk_i = bytes(rnd.getrandbits(8) for _ in range(32))
        a_i, pre_i, pk_i = keypair(sk_i)
        m_i = bytes(rnd.getrandbits(8) for _ in range(rnd.randrange(0, 300)))
        s_i = sign(a_i, pre_i, pk_i, m_i)
        c(f"random[{i}] honest", pk_i, s_i, True, m=m_i)
        x = bytearray(s_i); x[rnd.randrange(64)] ^= 1 << rnd.randrange(8)
        c(f"random[{i}] sig bit", pk_i, bytes(x), False, m=m_i)
        y = bytearray(pk_i); y[rnd.randrange(32)] ^= 1 << rnd.randrange(8)
        c(f"random[{i}] key bit", bytes(y), s_i, False, m=m_i)
        if m_i:
            z = bytearray(m_i); z[rnd.randrange(len(m_i))] ^= 1
            c(f"random[{i}] msg bit", pk_i, s_i, False, m=bytes(z))

    mine = bouchaud(oracle, cases)
    ossl = openssl(cases)
    sod = libsodium(cases)
    bad = 0
    for i, (name, _, _, _, exp) in enumerate(cases):
        b = mine[i]
        o = "-" if ossl is None else int(ossl[i])
        s = "-" if sod is None else int(sod[i])
        verdict = "ok" if b == exp else "DIVERGE"
        if b != exp or (sod is not None and b and not sod[i]):
            bad += 1
        print(f"ED25519_ADV bouchaud={int(b)} attendu={int(exp)} openssl={o} libsodium={s} {verdict:7s} {name}")
    print(f"ED25519_ADV_BILAN cas={len(cases)} divergences={bad} "
          f"openssl={'oui' if ossl is not None else 'absent'} libsodium={'oui' if sod is not None else 'absent'}")
    if strict and (ossl is None or sod is None):
        print("ED25519_ADV_ECHEC reference absente (cryptography et PyNaCl requis en --strict)")
        sys.exit(2)
    if strict and bad:
        sys.exit(1)


if __name__ == "__main__":
    main()
