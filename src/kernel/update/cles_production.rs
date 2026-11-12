// BOUCHAUD_CLES_PRODUCTION_V1 -- the PUBLIC halves of the production release
// keys this system trusts.
//
// EMPTY on purpose: no production key has been generated yet. With no key,
// every install is refused (`aucune_cle_de_confiance`) -- fail closed, never a
// silent fallback to a development key.
//
// Adding a key is a HUMAN decision (docs/current/TRIGKEY_INSTALL_UPDATE.md,
// "Actions humaines") :
//   1. generate the key pair OFFLINE (never on a CI runner, never committed):
//        openssl genpkey -algorithm ed25519 -out release-AAAA.pem
//   2. extract the public half:
//        python3 tools/release/cle_publique.py release-AAAA.pem --nom release-aaaa-mm
//   3. add it below in a pull request, with its name and the date;
//   4. store the private half as the `BOUCHAUD_RELEASE_ED25519_PEM` secret of
//      the protected GitHub environments `trigkey-testing` / `trigkey-stable`.
//
// Rotation: add the new key, ship a release that trusts both, then sign with
// the new one; revocation: remove the old key in a later release. Only public
// keys live here; `tools/verifie-cle-dev.py` refuses any private key in the
// repository.
pub const CLES_PRODUCTION: &[(&str, &str)] = &[
    // ("release-2026-xx", "<64 lowercase hex>"),
];
