//! Session Ladybird supervisee par le bureau. Les moteurs gardent leurs IPC natifs.
use core::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};
pub const DEMARRER: u8 = 1;
pub const ARRETER: u8 = 2;
static COMMANDE: AtomicU8 = AtomicU8::new(0);
static RACINE: AtomicU32 = AtomicU32::new(0);
static ECHEC: AtomicU8 = AtomicU8::new(0);
pub fn demande(action: u8) {
    COMMANDE.store(action, Ordering::Release);
    crate::kernel::sync::reveil::INTERFACE.signale(crate::kernel::sync::reveil::Source::Explicite);
}
pub fn prend_commande() -> u8 { COMMANDE.swap(0, Ordering::AcqRel) }
pub fn enregistre(pid: u32) { RACINE.store(pid, Ordering::Release); ECHEC.store(0, Ordering::Release); }
pub fn echec() { ECHEC.store(1, Ordering::Release); }
pub fn racine() -> u32 { RACINE.load(Ordering::Acquire) }
pub fn en_echec() -> bool { ECHEC.load(Ordering::Acquire) != 0 }
/// Arret borne de l'arbre, y compris les moteurs sans fenetre.
pub fn arrete() {
    let pid = RACINE.swap(0, Ordering::AcqRel);
    if pid != 0 {
        for child in crate::kernel::task::arbre_de(pid) {
            crate::kernel::task::tue_processus(child, 0);
        }
    }
}

// Reuse the existing five-second process sample, no additional MM scans on paint.
static CPU: AtomicU64 = AtomicU64::new(0);
static RSS: AtomicU64 = AtomicU64::new(0);
static SAMPLE_MS: AtomicU64 = AtomicU64::new(0);
/// Le nom de processus Ladybird correspondant a chaque service du registre.
///
/// # Ce tableau ne definit PLUS le contenu de la fenetre
///
/// Il le faisait : la vue Services parcourait cette liste et n'affichait
/// qu'elle -- six lignes, et rien du systeme ni du reseau. Il ne sert plus
/// qu'a une chose, la bonne : traduire un nom de processus en identifiant de
/// service, pour que les mesures CPU et RSS aillent alimenter le registre.
/// C'est le registre, et lui seul, qui decide ce que la fenetre montre.
/// Combien de fois le detail des fautes n'a pas pu etre lu.
///
/// Voir le commentaire au point de lecture : `try_lock` echoue d'autant plus
/// volontiers que le systeme faute, donc au pire moment. Ce compteur transforme
/// un trou silencieux dans la serie en un fait qu'on peut lire.
static RELEVES_FAUTES_MANQUES: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

const PROCESSUS_VERS_SERVICE: [(&str, &str); 6] = [
    ("BouchaudBrowserHost", "browser.host"),
    ("RequestServer", "browser.request_server"),
    ("WebContent", "browser.web_content"),
    ("ImageDecoder", "browser.image_decoder"),
    ("WebWorker", "browser.web_worker"),
    ("Compositor", "browser.compositor"),
];

pub fn observe(rows: &[crate::kernel::task::Mesure], window: u64) {
    use crate::kernel::services::{self, registre::Kpi, Etat};
    let root = racine();
    let mut ticks = 0u64;
    let mut rss = 0u64;
    for row in rows {
        if root != 0 && row.resource_group_id == root {
            ticks = ticks.saturating_add(row.ticks);
            rss = rss.saturating_add(row.rss_octets);
        }
    }
    CPU.store(if window == 0 { 0 } else { ticks.saturating_mul(100)/window }, Ordering::Relaxed);
    RSS.store(rss, Ordering::Relaxed);
    SAMPLE_MS.store(crate::kernel::timer::monotonic_ms(), Ordering::Release);

    // LES MESURES REJOIGNENT LE REGISTRE, au lieu de rester ici.
    //
    // Le releve existait deja -- cinq secondes, une seule passe sur les
    // processus. Il alimentait une fenetre qui ne montrait que Ladybird ; il
    // alimente maintenant les noeuds `browser.*` du registre, que la fenetre,
    // la ligne de commande et la boite noire lisent toutes les trois.
    // BOUCHAUD_C27_ARBRE_PAR_PROCESSUS
    //
    // Deux etages, et le second est le seul qui reponde a la question utile.
    //
    // L'etage AGREGE reste : « le navigateur prend-il 40 % ou 400 % ? » est
    // une question legitime et c'est la ligne du role qui y repond.
    //
    // L'etage INSTANCE est nouveau. Trois WebContent dont un sature un coeur
    // et deux dorment donnent EXACTEMENT le meme cumul que trois qui
    // travaillent au tiers -- et le remede n'est pas le meme. Il faut donc
    // une ligne par PID, sous la ligne du role, avec de quoi designer le
    // coupable : son coeur chaud, ses fils executables, ses fautes.
    //
    // Les identifiants d'instance sont `<service>.<pid>` et tiennent dans les
    // trente-deux octets de `Id` : `browser.web_content.4294967295` en fait
    // trente. Le registre en accepte cent vingt-huit ; le navigateur en
    // publie une poignee.
    let mut vus: [u32; INSTANCES_MAX] = [0; INSTANCES_MAX];
    let mut nombre_vus = 0usize;

    for (nom, service) in PROCESSUS_VERS_SERVICE {
        let mut instances = 0u32;
        let mut ticks = 0u64;
        let mut rss = 0u64;
        let mut vss = 0u64;
        let mut pid_unique = 0u32;
        let mut fautes = crate::kernel::fautes::Compte::default();

        for row in rows {
            if root != 0 && row.resource_group_id != root && row.pid != root {
                continue;
            }
            let base = row.nom.rsplit('/').next().unwrap_or(&row.nom);
            let correspond = base == nom || (nom == "BouchaudBrowserHost" && row.pid == root);
            if !correspond {
                continue;
            }
            instances += 1;
            pid_unique = row.pid;
            ticks = ticks.saturating_add(row.ticks);
            rss = rss.saturating_add(row.rss_octets);
            vss = vss.saturating_add(row.vss_octets);
            let compte_du_processus = crate::kernel::task::fautes_du_processus(row.pid);
            if let Some(compte) = compte_du_processus {
                fautes.fusionne(&compte);
            }

            // --- L'INSTANCE ---------------------------------------------
            if nombre_vus < INSTANCES_MAX {
                let mut id = [0u8; 40];
                let longueur = identifiant_instance(&mut id, service, row.pid);
                let Ok(texte) = core::str::from_utf8(&id[..longueur]) else { continue };
                services::declare(texte, service, services::Genre::Processus);

                // LE COEUR CHAUD, et pourquoi il ne se deduit pas du total.
                //
                // Un processus a 100 % reparti sur quatre coeurs et un
                // processus qui sature un seul coeur se lisent pareil dans un
                // pourcentage global. Le second est celui qui fait saccader :
                // c'est un seul fil qui travaille, et aucune machine plus
                // large ne le rendra plus rapide.
                let mut chaud = 0usize;
                let mut chaud_ns = 0u64;
                for (index, ns) in row.cpu_map_ns.iter().enumerate() {
                    if *ns > chaud_ns {
                        chaud_ns = *ns;
                        chaud = index;
                    }
                }
                let dominante = crate::kernel::task::faute_dominante(row.pid)
                    .map(|(categorie, _)| categorie.nom());

                // LA MEME MESURE, SUR LA CONSOLE SERIE.
                //
                // BOUCHAUD_C34_LES_FAUTES_PAR_PHASE
                //
                // Le registre porte deja ces chiffres, mais il se lit dans une
                // fenetre et dans la boite noire. Les bancs, eux, lisent le
                // journal serie -- et c'est la que se trouvent, a cote, les
                // `PERF_EXECVE` du noyau et les etapes horodatees du
                // navigateur. Sans cette ligne, on ne peut pas soustraire :
                // « les vingt-trois secondes avant `main` sont X ms d'image,
                // Y ms de fautes fichier, Z ms de fautes zero » demande que
                // les trois mesures soient dans le MEME journal.
                //
                // Le detail par categorie compte : « fichier » se soigne par
                // la lecture anticipee, « zero » par le dimensionnement des
                // arenes, « attente » par la contention. Un total ne dit
                // lequel des trois grandit.
                // UN RELEVE MANQUANT DOIT SE VOIR.
                //
                // `fautes_par_categorie` prend le livre des fautes par
                // `try_lock` -- et c'est le bon choix : un releve periodique
                // n'a pas a bloquer le chemin de faute. Mais l'echec est
                // SILENCIEUX, et il est d'autant plus probable que le systeme
                // faute beaucoup, c'est-a-dire exactement quand la ligne
                // interesse. Une instrumentation muette au moment ou elle
                // compte ne vaut pas mieux que pas d'instrumentation.
                // BOUCHAUD_C56_LIRE_AVANT_LA_MORT
                //
                // `FAULT_FILE_BREAKDOWN` ne sort qu'a la SORTIE du processus.
                // Or les WebWorker restent vivants : dans le run 35888970521,
                // aucun n'a produit de ligne par PID, et ce sont justement eux
                // le sujet. Seul WebContent en a emis -- et trois fois, donc a
                // chaque sortie de THREAD, pas de processus.
                //
                // Ce releve-ci est un SNAPSHOT en lecture seule du meme livre,
                // pris au rythme du sondeur. Il ne remet rien a zero. Avec
                // l'horloge partagee, il se recoupe avec les `WORKER_ETAPE` et
                // borne donc les intervalles pre-`main` sans avoir a attendre
                // la mort du processus.
                //
                // `user_ms`/`sys_ms` viennent du livre /proc deja en place :
                // `wall - (user + sys)` dit ce qui s'est passe HORS processeur,
                // et c'est la question ouverte des 88,5 s de pid=16.
                if let Some(ph) = crate::kernel::task::phases_fichier_du_processus(row.pid) {
                    let (ph, perdues_global) = ph;
                    if ph.nombre != 0 {
                        let (user_ms, sys_ms, fils_morts) =
                            match crate::kernel::task::proc_processus_cumul(row.pid) {
                                Some(c) => (
                                    c.user_ns / 1_000_000,
                                    c.system_ns / 1_000_000,
                                    c.threads_zombies,
                                ),
                                None => (0, 0, 0),
                            };
                        crate::kernel::dmesg::log_fmt(format_args!(
                            "FAULT_FILE_SNAPSHOT t={} pid={} image={} faults={} \
total_us={} wait_us={} acquire_us={} hit_n={} miss_n={} miss_us={} \
miss_read_us={} direct_us={} mm_us={} map_us={} explained_us={} \
residual_us={} residual_pct={} user_ms={} sys_ms={} fils_morts={} \
lost_samples_global={}",
                            crate::kernel::timer::monotonic_ms(),
                            row.pid,
                            base,
                            ph.nombre,
                            ph.total_ns / 1_000,
                            ph.attente_ns / 1_000,
                            ph.acquire_ns / 1_000,
                            ph.hit_n,
                            ph.miss_n,
                            ph.miss_ns / 1_000,
                            // DONT de miss_us : ne jamais additionner.
                            ph.acquire_backing_ns / 1_000,
                            ph.backing_direct_ns / 1_000,
                            ph.mm_ns / 1_000,
                            ph.map_ns / 1_000,
                            ph.explique_ns() / 1_000,
                            ph.residu_ns() / 1_000,
                            ph.residu_pct(),
                            user_ms,
                            sys_ms,
                            // Combien de threads MORTS ce total comprend.
                            // Voir BOUCHAUD_C69 : sans eux il reculait.
                            fils_morts,
                            // Compteur GLOBAL : pas « ce pid a perdu N ».
                            perdues_global,
                        ));
                    }
                }

                let detail = crate::kernel::task::fautes_par_categorie(row.pid);
                if detail.is_none() {
                    RELEVES_FAUTES_MANQUES.fetch_add(1, Ordering::Relaxed);
                }
                if let Some(par_categorie) = detail {
                    use crate::kernel::fautes::Categorie;
                    let c = |categorie: Categorie| par_categorie[categorie.rang()];
                    let zero = c(Categorie::Zero);
                    let fichier = c(Categorie::FichierPrive);
                    let partage = c(Categorie::Partage);
                    let attente = c(Categorie::Attente);
                    let copie = c(Categorie::Copie);
                    let echec = c(Categorie::Echec);
                    let total = compte_du_processus.unwrap_or_default();
                    crate::serial_println!(
                        "[PERF-PROC] t={} pid={} image={} rss_kio={} vss_kio={} taches={} \
fautes={} total_us={} pire_us={} \
zero={}/{}us fichier={}/{}us partage={}/{}us copie={}/{}us attente={}/{}us echec={}",
                        crate::kernel::timer::monotonic_ms(),
                        row.pid,
                        base,
                        row.rss_octets / 1024,
                        row.vss_octets / 1024,
                        row.taches,
                        total.nombre,
                        total.total_ns / 1_000,
                        total.pire_ns / 1_000,
                        zero.nombre, zero.total_ns / 1_000,
                        fichier.nombre, fichier.total_ns / 1_000,
                        partage.nombre, partage.total_ns / 1_000,
                        copie.nombre, copie.total_ns / 1_000,
                        attente.nombre, attente.total_ns / 1_000,
                        echec.nombre,
                    );
                }

                services::kpi(
                    texte,
                    Kpi {
                        cpu_pour_mille: Some(if window == 0 {
                            0
                        } else {
                            (row.ticks.saturating_mul(1000) / window) as u32
                        }),
                        rss_octets: Some(row.rss_octets),
                        vss_octets: Some(row.vss_octets),
                        pid: Some(row.pid),
                        ppid: Some(row.ppid),
                        groupe: Some(row.resource_group_id),
                        threads: Some(row.taches),
                        threads_executables: Some(row.runnable_threads),
                        cpu_chaud: if chaud_ns == 0 { None } else { Some(chaud) },
                        cpu_chaud_pour_mille: if window == 0 || chaud_ns == 0 {
                            None
                        } else {
                            Some((chaud_ns.saturating_mul(1000) / window) as u32)
                        },
                        migrations: Some(row.migrations),
                        commutations: Some(row.context_switches),
                        // `None` et non `Some(0)` : le livre des fautes est
                        // borne et il chasse. Un zero affirmerait « ce
                        // processus ne faute pas » la ou le livre ne sait
                        // simplement rien de lui.
                        fautes_nombre: compte_du_processus.map(|c| c.nombre),
                        fautes_total_us: compte_du_processus.map(|c| c.total_ns / 1_000),
                        fautes_pire_us: compte_du_processus.map(|c| c.pire_ns / 1_000),
                        fautes_dominante: dominante,
                        ..Kpi::default()
                    },
                );
                services::etat(texte, Etat::Actif);
                vus[nombre_vus] = row.pid;
                nombre_vus += 1;
            }
        }

        // --- LE ROLE, agrege ------------------------------------------
        if instances > 0 {
            services::kpi(
                service,
                Kpi {
                    cpu_pour_mille: Some(if window == 0 {
                        0
                    } else {
                        (ticks.saturating_mul(1000) / window) as u32
                    }),
                    rss_octets: Some(rss),
                    vss_octets: Some(vss),
                    pid: if instances == 1 { Some(pid_unique) } else { None },
                    instances: Some(instances),
                    fautes_nombre: if fautes.vide() { None } else { Some(fautes.nombre) },
                    fautes_total_us: if fautes.vide() { None } else { Some(fautes.total_ns / 1_000) },
                    fautes_pire_us: if fautes.vide() { None } else { Some(fautes.pire_ns / 1_000) },
                    ..Kpi::default()
                },
            );
            services::etat(service, Etat::Actif);
        } else {
            // ARRETE, ET TOUJOURS DANS L'ARBRE. Un `WebWorker` a la demande
            // doit se lire « arrete », pas disparaitre.
            services::kpi(service, Kpi::default());
            services::etat(service, Etat::Arrete);
        }
    }

    // LES INSTANCES DISPARUES DOIVENT LE DIRE.
    //
    // Un processus qui meurt laisserait sinon sa derniere ligne affichee pour
    // toujours -- 83 % de CPU fige, sur un PID qui n'existe plus. Celles qui
    // n'ont pas ete revues dans cette passe passent a `Arrete` avec des
    // mesures vides ; elles restent dans l'arbre, parce qu'un WebContent qui
    // vient de mourir est exactement ce qu'on cherche apres un plantage.
    services::oublie_instances_absentes(&vus[..nombre_vus]);
}

/// Combien d'instances de processus navigateur la fenetre peut suivre.
///
/// Le registre en accepte cent vingt-huit en tout, systeme et reseau compris.
/// Trente-deux instances de navigateur, c'est deja plus d'onglets que la
/// machine n'en tiendra ; la borne existe pour que la table ne se remplisse
/// jamais au point de faire disparaitre le reste de l'arbre.
const INSTANCES_MAX: usize = 32;

/// Ecrit `<service>.<pid>` dans `sortie` et rend sa longueur.
///
/// Sans `alloc` : ce chemin tourne a chaque releve, et un `String` par
/// processus et par passe serait une allocation par onglet toutes les cinq
/// secondes pour un identifiant qui ne change jamais.
fn identifiant_instance(sortie: &mut [u8; 40], service: &str, pid: u32) -> usize {
    let mut n = 0;
    for octet in service.as_bytes() {
        if n == sortie.len() { return n; }
        sortie[n] = *octet;
        n += 1;
    }
    if n < sortie.len() {
        sortie[n] = b'.';
        n += 1;
    }
    let mut chiffres = [0u8; 10];
    let mut combien = 0;
    let mut reste = pid;
    if reste == 0 {
        chiffres[0] = b'0';
        combien = 1;
    }
    while reste > 0 {
        chiffres[combien] = b'0' + (reste % 10) as u8;
        combien += 1;
        reste /= 10;
    }
    while combien > 0 && n < sortie.len() {
        combien -= 1;
        sortie[n] = chiffres[combien];
        n += 1;
    }
    n
}

/// Prend le releve des processus s'il est temps, et publie l'arbre.
///
/// BOUCHAUD_C27_MESURE_SANS_BUREAU
///
/// Le releve n'etait declenche QUE par la boucle du gestionnaire de fenetres.
/// Sur une machine sans affichage -- la CI, une session serie, l'enregistreur
/// de vol sur la TRIGKEY -- cette boucle ne peint pas, et le releve n'avait
/// tout simplement jamais lieu. La mesure le montre sans ambiguite : sur un
/// demarrage complet en `-display none`, `observe()` a ete appele UNE fois,
/// celle que la commande `services` a provoquee elle-meme.
///
/// Consequence : l'arbre des processus etait vide precisement dans le cas ou
/// l'on diagnostique a distance, c'est-a-dire celui qui compte.
///
/// La cadence reste celle d'avant -- cinq secondes -- et elle est PARTAGEE :
/// `observe()` date chaque passe, et cette fonction ne prend un releve que si
/// personne ne l'a fait depuis. Les compteurs de `mesure_processus` sont
/// differentiels ; deux preneurs non coordonnes se voleraient leurs deltas et
/// afficheraient chacun la moitie du CPU reel.
static PASSES_SONDE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

pub fn releve_si_du() {
    const PERIODE_MS: u64 = 5_000;
    let maintenant = crate::kernel::timer::monotonic_ms();
    let dernier = SAMPLE_MS.load(Ordering::Acquire);
    if dernier != 0 && maintenant.saturating_sub(dernier) < PERIODE_MS {
        return;
    }
    let (mesures, total) = crate::kernel::task::mesure_processus();
    observe(&mesures, total);

    // BOUCHAUD_C68_LES_SONDES_ETAIENT_AU_MAUVAIS_ENDROIT
    //
    // Ces trois lignes etaient emises depuis `PROCESS_EXIT`. Sur le banc
    // local elles sortaient ; au run 35948934416 le bloc de preuves a rendu
    // « une famille absente n'a pas ete emise par ce noyau » pour les trois,
    // parce que sous Ladybird AUCUN service ne meurt pendant la fenetre
    // mesuree. Une sonde accrochee a la mort ne mesure rien d'un processus
    // vivant, et c'est exactement le processus qu'on cherche a expliquer.
    //
    // Elles partent donc de l'echantillonneur, comme `FAULT_FILE_SNAPSHOT`
    // qui, lui, est bien sorti.
    //
    // CADENCE : une fois par minute, pas une fois par passe. L'echantillonneur
    // tourne toutes les cinq secondes ; sur un smoke de trois cent quarante
    // secondes, publier a chaque passe noierait le bloc de preuves sous huit
    // cents lignes. Ces compteurs sont CUMULATIFS -- un releve par minute
    // suffit a voir la pente, et le dernier porte le total.
    const PASSES_PAR_SONDE: u64 = 12;
    let passe = PASSES_SONDE.fetch_add(1, Ordering::Relaxed);
    if passe % PASSES_PAR_SONDE == 0 {
    let (replie_user, replie_noyau) = crate::kernel::task::proc_cpu_compteurs();
    crate::kernel::dmesg::log_fmt(format_args!(
        "CPU_CUMUL scope=global t={} replie_user_ms={} replie_noyau_ms={}",
        maintenant,
        replie_user / 1_000_000,
        replie_noyau / 1_000_000,
    ));
    let (bal_appels, bal_entrees, bal_ns, bal_pire, bal_candidats) =
        crate::kernel::clean_page_cache::balayage_stats();
    crate::kernel::dmesg::log_fmt(format_args!(
        "CACHE_BALAYAGE scope=global t={} appels={} entrees_parcourues={} \
total_us={} pire_us={} candidats_suffisants={}",
        maintenant, bal_appels, bal_entrees, bal_ns / 1_000, bal_pire / 1_000, bal_candidats,
    ));
    let (evites, en_table, recuperees) =
        crate::kernel::clean_page_cache::balayage_temoins();
    crate::kernel::dmesg::log_fmt(format_args!(
        "CACHE_BALAYAGE_TEMOINS scope=global evites={} entrees={} recuperees={}",
        evites, en_table, recuperees,
    ));
    let (yields, pire_chaine, reprises, chaines) = crate::kernel::task::fault_retry_cumul();
    crate::kernel::dmesg::log_fmt(format_args!(
        "FAULT_REPRISE scope=global t={} yields={} pire_chaine={} reprises={} chaines={}",
        maintenant, yields, pire_chaine, reprises, chaines,
    ));
    crate::kernel::task::publie_syscall_top(8);
    }

    // LES DEUX CALCULS DE `/proc/stat`, COTE A COTE.
    //
    // BOUCHAUD_C29_PROC_STAT_CUMULATIF
    //
    // Sans cette ligne, « le total ne recule pas » et « le total est juste »
    // se confondent. Le defaut ne se demontre pas en guettant une baisse : la
    // croissance des autres taches la masque, et vingt echantillons
    // consecutifs peuvent rester monotones sans rien prouver.
    //
    //   cumulatif      ce que `/proc/stat` publie, alimente a l'imputation
    //   somme_vivants  ce qu'il publiait : la somme des taches encore en vie
    //   temps_mort     ce que la seconde a deja perdu, a la nanoseconde
    //
    // `temps_mort` non nul veut dire que le second calcul ment deja.
    let (vivants_user, vivants_sys) = crate::kernel::task::proc_cpu_somme_vivants();
    let (compteurs_user, compteurs_sys) = crate::kernel::task::proc_cpu_compteurs();
    // LES DEUX FACONS DE QUITTER LA SOMME DES VIVANTS, LUES ET NON PHOTOGRAPHIEES.
    let (zombies_ns, zombies, vivantes) = crate::kernel::task::proc_cpu_zombies();
    let recycle = crate::kernel::task::proc_temps_recycle_ns();
    // L'ECART, ET CE QU'IL RESTE QUAND ON A NOMME SES CAUSES.
    //
    // BOUCHAUD_C33_L_ECART_INEXPLIQUE
    //
    // Mesure precedente : `temps_mort_ms=18` pour un ecart de 1113 ms. Dix-huit
    // millisecondes sur mille cent treize -- un virgule six pour cent. Publier
    // l'ecart sans publier ce qui en reste inexplique laissait croire qu'il
    // etait compris.
    //
    // `residu` est la partie que NI la mort NI le recyclage n'expliquent. Tant
    // qu'il n'est pas petit, une troisieme sortie de l'ensemble des vivants
    // existe et n'a pas ete trouvee ; le banc echoue dessus plutot que de la
    // taire.
    let ecart = compteurs_user
        .saturating_add(compteurs_sys)
        .saturating_sub(vivants_user.saturating_add(vivants_sys));
    let residu = ecart.saturating_sub(zombies_ns.saturating_add(recycle));
    let c = crate::kernel::task::proc_cpu_cumul();
    // TROIS VALEURS INDEPENDANTES : voir `proc_cpu_compteurs`.
    //
    //   publie          ce que `/proc/stat` rend reellement
    //   cumulatif       ce que les compteurs persistants disent
    //   somme_vivants   ce que rendrait la somme des taches en vie
    //
    // `publie` doit suivre `cumulatif`. S'il suit `somme_vivants`, le temps
    // des taches mortes est perdu -- et la comparaison le dit, meme si
    // `proc_cpu_cumul` a ete ramene au calcul d'avant.
    crate::serial_println!(
        "[PROC-STAT] publie_user_ms={} publie_sys_ms={} \
cumulatif_user_ms={} cumulatif_sys_ms={} \
somme_vivants_user_ms={} somme_vivants_sys_ms={} \
zombies_ms={} temps_recycle_ms={} ecart_ms={} residu_ms={}",
        c.user_ns / 1_000_000,
        c.system_ns / 1_000_000,
        compteurs_user / 1_000_000,
        compteurs_sys / 1_000_000,
        vivants_user / 1_000_000,
        vivants_sys / 1_000_000,
        zombies_ns / 1_000_000,
        recycle / 1_000_000,
        ecart / 1_000_000,
        residu / 1_000_000,
    );
    crate::serial_println!(
        "[PROC-STAT] zombies={} vivantes={} releves_fautes_manques={}",
        zombies,
        vivantes,
        RELEVES_FAUTES_MANQUES.load(Ordering::Relaxed),
    );
    let (depassements, pire) = crate::kernel::task::proc_depassements();
    crate::serial_println!(
        "[PROC-STAT] capacite_ms={} occupe_ms={} depassements={} pire_depassement_ms={}",
        (crate::kernel::timer::monotonic_ns() / 1_000_000)
            .saturating_mul(c.online as u64),
        (c.user_ns.saturating_add(c.system_ns)) / 1_000_000,
        depassements,
        pire / 1_000_000,
    );
}

// BOUCHAUD_V13_SERVICES_SAMPLER_DEDIE
// Le releve des processus n'appartient ni au rendu ni au pilote USB.
// Ce petit fil generique le rend disponible en GUI, en headless et dans la
// blackbox, sans faire dependre l'observabilite du lifecycle xHCI.
static FIL_MESURES_PROCESSUS: AtomicU8 = AtomicU8::new(0);

fn fil_mesures_processus() -> ! {
    loop {
        releve_si_du();
        // BOUCHAUD_C72_CHECKPOINT_FAIL_SAFE
        //
        // Ce fil-ci et pas un autre : ni le compositeur, ni le reseau, ni le
        // pilote USB ne doivent porter la persistance du diagnostic. La
        // fonction se borne elle-meme et rend la main immediatement sans
        // support.
        crate::kernel::blackbox::checkpoint_si_du();
        // La fonction elle-meme borne la vraie mesure a une fois / 5 s.
        // 100 ms ne sert qu'a ne pas retarder le premier echantillon.
        crate::kernel::task::sleep_ticks(100);
    }
}

pub fn demarre_fil_mesures_processus() -> bool {
    if FIL_MESURES_PROCESSUS
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return true;
    }
    if crate::kernel::task::spawn_noyau_priorite(
        fil_mesures_processus,
        "services-metrics",
        crate::kernel::task::Priorite::Normale,
    ) {
        crate::serial_println!("BOUCHAUD_SERVICES_METRICS_FIL_LANCE periode_ms=5000");
        true
    } else {
        FIL_MESURES_PROCESSUS.store(0, Ordering::Release);
        crate::serial_println!("BOUCHAUD_SERVICES_METRICS_FIL_REFUSE");
        false
    }
}

pub fn usage() -> (u64, u64, u64) {
    let sample = SAMPLE_MS.load(Ordering::Acquire);
    (CPU.load(Ordering::Relaxed), RSS.load(Ordering::Relaxed), sample)
}
