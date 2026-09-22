# Fixtures navigateur autonomes (V13)

Ces quatre fichiers sont des fixtures de test **versionnees dans Bouchaud OS**.
Elles rendent `tools/health/images_fixtures.py` independant d'un checkout de
`third_party/ladybird`, afin que CI Fast et Reliability puissent verifier le
banc sans telecharger Ladybird.

Generation de reference (Pillow 12.3.0) : images RGB 32x32 deterministes,
JPEG baseline 4:4:4 et WebP lossless/lossy. Le runtime de test n'a aucune
dependance a Pillow : les octets sont commits tels quels et leur SHA-256 est
verifie par `test_images_fixtures.py`.

Le navigateur reste juge par ses propres decodeurs : la page charge ces octets,
les dessine dans un canvas et relit des pixels connus. Les fixtures ne servent
pas a remplacer les tests upstream Ladybird, seulement a rendre le smoke test
autonome et reproductible.

Voir `manifest.json` pour tailles, dimensions, pixels de reference et SHA-256.
