// Ou poser les partitions sur le disque interne, et rien d'autre.
//
// Cette arithmetique decide de tout et ne peut pas etre mise au point sur la
// cible : elle s'execute une fois, elle detruit ce qui etait la, et si elle est
// fausse la machine ne demarre plus. Elle ne connait donc ni le NVMe, ni
// l'archive, ni le micrologiciel -- seulement des nombres --, ce qui permet a
// `tools/platform/test_installation.rs` de lui donner les geometries qu'on ne
// possede pas : un disque de 512 Go en blocs de 4096, un disque presque plein,
// un disque trop petit.

/// Taille minimale de l'ESP, en mebioctets.
///
/// Ce n'est pas un confort. Sous 65 525 amas, un volume n'est pas du FAT32, et
/// un micrologiciel conforme le lit comme du FAT16 -- ou il ne trouve rien.
pub const ESP_MINIMUM_MIO: u64 = 512;

/// Secteurs de 512 octets que la zone de persistance occupe.
///
/// Elle doit rester d'accord avec `SECTEURS_ZONE` de `fs::persistance` : ce
/// module ne peut pas l'importer -- il est inclus tel quel dans un test hote
/// qui n'a pas le noyau --, alors `tools/verifie-installation.py` verifie que
/// les deux nombres sont les memes.
pub const SECTEURS_PERSISTANCE: u64 = 262_144;

/// Marge laissee au-dessus de la charge utile.
///
/// Une ESP calee au plus juste devient impossible a mettre a jour : la
/// prochaine version du noyau ne rentrera pas, et il faudra repartitionner un
/// disque qui porte deja les donnees de l'utilisateur.
pub const ESP_MARGE_MIO: u64 = 256;

/// Ou vont les deux partitions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub esp_premier: u64,
    /// Dernier bloc de l'ESP, INCLUS.
    pub esp_dernier: u64,
    pub systeme_premier: u64,
    /// Dernier bloc de la partition systeme, INCLUS.
    pub systeme_dernier: u64,
}

impl Plan {
    pub const fn esp_blocs(&self) -> u64 {
        self.esp_dernier - self.esp_premier + 1
    }
    pub const fn systeme_blocs(&self) -> u64 {
        self.systeme_dernier - self.systeme_premier + 1
    }
}

/// Blocs par mebioctet, pour une taille de bloc donnee.
pub const fn blocs_par_mio(taille_bloc: usize) -> u64 {
    let par = 1024 * 1024 / taille_bloc;
    if par == 0 { 1 } else { par as u64 }
}

/// Aligne un bloc sur la frontiere de mebioctet superieure.
///
/// Un SSD qui recoit des ecritures desalignees sur ses pages internes fait des
/// lecture-modification-ecriture invisibles : la partition marche, et elle est
/// durablement plus lente. Le defaut ne se voit jamais ; il se mesure.
pub const fn aligne(bloc: u64, taille_bloc: usize) -> u64 {
    let par_mio = blocs_par_mio(taille_bloc);
    let reste = bloc % par_mio;
    if reste == 0 { bloc } else { bloc + (par_mio - reste) }
}

/// Calcule ou poser les deux partitions.
///
/// `premier_utilisable` et `dernier_utilisable` viennent de la disposition GPT :
/// ce sont les bornes hors tables. `octets_utiles` est ce que l'ESP doit
/// porter.
///
/// Rend `None` quand le disque ne peut pas porter les deux. Refuser vaut mieux
/// que produire un plan ou la partition systeme fait quelques blocs : elle
/// serait acceptee, formatee, et la persistance n'y tiendrait pas -- une
/// installation reussie sur laquelle rien ne se sauvegarde.
pub fn plan(
    premier_utilisable: u64,
    dernier_utilisable: u64,
    taille_bloc: usize,
    octets_utiles: u64,
) -> Option<Plan> {
    if taille_bloc == 0 || premier_utilisable > dernier_utilisable {
        return None;
    }
    let par_mio = blocs_par_mio(taille_bloc);
    let mio_utiles = octets_utiles.div_ceil(1024 * 1024);
    let esp_mio = mio_utiles.checked_add(ESP_MARGE_MIO)?.max(ESP_MINIMUM_MIO);

    let esp_premier = aligne(premier_utilisable, taille_bloc);
    let esp_blocs = esp_mio.checked_mul(par_mio)?;

    // L'ESP ne prend jamais plus du QUART de la zone utilisable.
    //
    // Sans ce plafond, une charge annoncee absurde -- un compte de taille qui
    // deborde, une archive corrompue dont la longueur est un nombre au hasard
    // -- produit un plan arithmetiquement valide ou l'ESP mange le disque et
    // ou la partition systeme est un residu. Le plan serait accepte, la table
    // ecrite, et l'utilisateur decouvrirait apres coup qu'il n'a plus de place
    // pour ses donnees.
    //
    // Un quart est large : la charge reelle fait quelques centaines de
    // mebioctets, et le plancher de 512 Mio garde le dessus sur les petits
    // disques -- c'est pourquoi le plafond ne s'applique qu'au-dela.
    let zone = dernier_utilisable.checked_sub(esp_premier)?.checked_add(1)?;
    if esp_blocs > ESP_MINIMUM_MIO * par_mio && esp_blocs > zone / 4 {
        return None;
    }

    let esp_dernier = esp_premier.checked_add(esp_blocs)?.checked_sub(1)?;
    if esp_dernier >= dernier_utilisable {
        return None;
    }

    let systeme_premier = aligne(esp_dernier + 1, taille_bloc);

    // La partition systeme doit pouvoir porter la ZONE de persistance, et pas
    // seulement exister.
    //
    // `persistance` occupe 262 144 secteurs de 512 octets -- 128 Mio -- a la
    // FIN de son volume, et refuse de se monter si le volume est plus petit
    // que la zone augmentee de son contenu. Une partition de quatre-vingts
    // mebioctets serait donc creee, formatee, declaree installee, et rien ne
    // s'y sauvegarderait jamais : l'utilisateur aurait une installation
    // reussie sur laquelle chaque `sync` echoue en silence.
    //
    // On exige le quadruple de la zone. Le facteur laisse de la place au
    // CONTENU, qui vit dans la meme partition ; exiger la zone toute juste
    // donnerait un volume ou l'on peut ecrire un superbloc et rien d'autre.
    let zone_octets = SECTEURS_PERSISTANCE * 512 * 4;
    let systeme_minimum = zone_octets.div_ceil(taille_bloc as u64);
    if systeme_premier.checked_add(systeme_minimum)? > dernier_utilisable {
        return None;
    }
    Some(Plan {
        esp_premier,
        esp_dernier,
        systeme_premier,
        systeme_dernier: dernier_utilisable,
    })
}

/// Les deux partitions se recouvrent-elles ?
///
/// La verification vit ici et non seulement dans `gpt::ecrit_table` : un plan
/// faux doit etre refusable AVANT qu'on demande a la table de le refuser, pour
/// que le test hote puisse l'affirmer sans fabriquer un disque.
pub const fn se_recouvrent(plan: &Plan) -> bool {
    plan.esp_premier <= plan.systeme_dernier && plan.systeme_premier <= plan.esp_dernier
}
