//! Taxonomie des balises HTML5. Recense les familles utiles au layout : balises
//! vides (auto-fermantes), blocs (display:block par défaut), et catégories
//! sémantiques. Étendu pour couvrir le HTML moderne employé par les frameworks
//! (sections, formulaires, tableaux, média, éléments interactifs).

/// Balise vide (sans contenu / auto-fermante).
pub fn is_void(tag: &str) -> bool {
    matches!(tag,
        "area"|"base"|"br"|"col"|"embed"|"hr"|"img"|"input"|"link"|"meta"|
        "param"|"source"|"track"|"wbr")
}

/// Balise de bloc (display:block par défaut). Sert à la fermeture implicite des
/// `<p>` et au choix du mode de mise en page. Couvre tout le HTML5 sémantique.
pub fn is_block(tag: &str) -> bool {
    matches!(tag,
        // texte / structure de flux
        "p"|"div"|"section"|"article"|"header"|"footer"|"nav"|"main"|"aside"|
        "blockquote"|"pre"|"figure"|"figcaption"|"address"|"hgroup"|
        // titres
        "h1"|"h2"|"h3"|"h4"|"h5"|"h6"|
        // listes
        "ul"|"ol"|"li"|"dl"|"dt"|"dd"|"menu"|
        // tableaux (conteneurs de bloc)
        "table"|"thead"|"tbody"|"tfoot"|"tr"|"caption"|"colgroup"|
        // formulaires
        "form"|"fieldset"|"legend"|
        // interactif / divers
        "details"|"summary"|"dialog"|"hr"|
        // racine / document
        "title"|"body"|"html"|"head"|"center")
}

/// Balise inline courante (display:inline par défaut). Tout le reste hérite/par
/// défaut inline ; cette liste sert surtout de référence.
pub fn is_inline(tag: &str) -> bool {
    matches!(tag,
        "a"|"span"|"b"|"strong"|"i"|"em"|"u"|"s"|"small"|"big"|"sub"|"sup"|
        "mark"|"abbr"|"cite"|"code"|"kbd"|"samp"|"var"|"q"|"time"|"data"|
        "label"|"output"|"bdi"|"bdo"|"ruby"|"rt"|"rp"|"wbr"|"br"|"font"|"tt"|
        "ins"|"del")
}

/// Élément de métadonnée du `<head>` (non rendu).
pub fn is_metadata(tag: &str) -> bool {
    matches!(tag, "base"|"link"|"meta"|"style"|"title"|"head"|"noscript"|"template")
}

/// Élément de tableau (traité par le moteur de tableaux).
pub fn is_table_part(tag: &str) -> bool {
    matches!(tag, "table"|"thead"|"tbody"|"tfoot"|"tr"|"td"|"th"|"caption"|"col"|"colgroup")
}

/// Contrôle de formulaire (rendu interactif : champ, bouton, case…).
pub fn is_form_control(tag: &str) -> bool {
    matches!(tag, "input"|"textarea"|"select"|"button"|"option"|"optgroup"|"datalist"|"output"|"progress"|"meter")
}

/// Élément média temporel (substitut A/V).
pub fn is_media(tag: &str) -> bool {
    matches!(tag, "video"|"audio"|"source"|"track")
}

/// Échelle de titre (`<h1>`..`<h6>`) en niveaux de scale du moteur de texte.
pub fn heading_scale(tag: &str) -> Option<usize> {
    match tag { "h1" => Some(4), "h2" => Some(3), "h3" => Some(3), "h4" | "h5" | "h6" => Some(2), _ => None }
}
