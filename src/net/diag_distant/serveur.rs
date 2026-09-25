//! Le serveur BRDP : une pile smoltcp a NOUS, sur l'adresse de diagnostic.
//!
//! # Pourquoi une seconde interface plutot qu'une chaussette de plus
//!
//! La pile du navigateur et celle du debugger ne doivent pas se voir. Une
//! chaussette de plus sur l'interface existante les remettrait sur le meme
//! contexte : la meme table ARP, la meme adresse, la meme decision sur ce
//! qu'il faut refuser. Et refuser, pour TCP, veut dire `RST`.
//!
//! Deux interfaces, deux adresses, deux files. Le tri
//! (`diag_distant::tri`) decide qui recoit quoi AVANT que l'une ou l'autre
//! voie la trame ; chacune ne voit donc que ce qui la concerne, et aucune
//! n'a d'occasion de repondre a la place de l'autre.
//!
//! Cette interface herite en prime de smoltcp ce qu'on n'a pas a ecrire :
//! elle repond aux requetes ARP visant l'adresse LAB et aux `ping`. Le
//! premier geste de quiconque cherche a savoir si la machine est vivante
//! fonctionne donc sans une ligne de plus.
//!
//! # Sans jeton, ce serveur ne demarre pas
//!
//! `BOUCHAUD_DEBUG_TOKEN` est lu a la construction. Absent, `demarre()` rend
//! faux et rien n'ecoute. C'est un choix, et il est documente : un debugger
//! en lecture seule sans authentification reste un debugger qui publie
//! l'etat interne de la machine a quiconque atteint le segment local, et
//! « lecture seule » ne rend pas cela acceptable.
//!
//! ## « Desactive » veut dire ABSENT, pas endormi
//!
//! `demarre_services()` consulte `arme()` AVANT d'appeler `demarre()`. Comme
//! `arme()` se reduit a une constante, le compilateur elimine l'appel dans une
//! image sans jeton -- et avec lui le fil, la boucle d'acceptation et jusqu'au
//! nom `bouchaud-brdp`. Une image de production ne porte aucun code d'ecoute
//! BRDP, ce qui est une forme de desactivation qu'on ne peut pas mettre a
//! l'envers par erreur. `tools/ci/run_brdp_jeton.sh` le prouve sur le binaire.
//!
//! ## CE QUE LE JETON NE PROTEGE PAS
//!
//! Il est compile DANS l'image de laboratoire, en clair : `strings` sur cette
//! image le retrouve. C'est inherent a un secret partage lu a la construction,
//! et cela n'a rien d'un oubli -- mais il faut le dire, parce que cela fixe la
//! regle d'usage : UNE IMAGE LAB NE SE DISTRIBUE PAS. Elle est un outil de
//! banc. Ce que le jeton ferme, c'est l'acces depuis le segment local a
//! quelqu'un qui n'a PAS l'image ; il ne ferme rien a quelqu'un qui l'a.
//!
//! Ce que le protocole protege, en revanche, et qui tient meme sur un segment
//! ecoute : le jeton ne traverse JAMAIS le reseau. Le client prouve qu'il le
//! connait par un HMAC du nonce, et `tools/verifie-jeton-brdp.py` refuse tout
//! chemin qui le ferait sortir -- impression, boite noire, reponse, telemetrie.
//!
//! # Ce fil ne bloque personne
//!
//! Il possede sa propre file de trames, sa propre interface et ses propres
//! tampons. Il ne prend jamais `VERROU_RECEPTION` -- c'est le drainage qui
//! lui depose ses trames -- et il ne tient aucun verrou de la boite noire
//! pendant qu'il ecrit sur le reseau.

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::phy::{Device, DeviceCapabilities, Medium};
use smoltcp::socket::tcp;
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr};

use super::brdp::{self, Commande, Decoupeur, Erreur};
use super::reponses::{self, Reponse};
use super::session::{self, FileCommandes, ModeEvenements, LIGNE_TROP_LONGUE};

/// Le jeton, injecte au build. JAMAIS commite, jamais ecrit dans la boite
/// noire, jamais renvoye sur le fil.
const JETON: Option<&str> = option_env!("BOUCHAUD_DEBUG_TOKEN");

const TRAME_MAX: usize = 1600;

static LANCE: AtomicBool = AtomicBool::new(false);
static CONNEXIONS: AtomicU64 = AtomicU64::new(0);
static AUTH_OK: AtomicU64 = AtomicU64::new(0);
static AUTH_KO: AtomicU64 = AtomicU64::new(0);
static COMMANDES: AtomicU64 = AtomicU64::new(0);
static REFUSEES: AtomicU64 = AtomicU64::new(0);
static OCTETS_RENDUS: AtomicU64 = AtomicU64::new(0);
/// Sessions dont le pair a ferme son sens RX.
static FERMETURES_DISTANTES: AtomicU64 = AtomicU64::new(0);

/// Le peripherique de la pile de diagnostic.
///
/// Il lit la file que le tri remplit, et emet directement par la carte : le
/// sens TX n'a pas de probleme de propriete -- une trame emise n'est retiree
/// a personne.
struct Peripherique;

struct JetonRx {
    tampon: [u8; TRAME_MAX],
    n: usize,
}

struct JetonTx;

impl smoltcp::phy::RxToken for JetonRx {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        f(&self.tampon[..self.n])
    }
}

impl smoltcp::phy::TxToken for JetonTx {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, taille: usize, f: F) -> R {
        let mut tampon = [0u8; TRAME_MAX];
        let n = taille.min(TRAME_MAX);
        let r = f(&mut tampon[..n]);
        crate::drivers::e1000::send(&tampon[..n]);
        r
    }
}

impl Device for Peripherique {
    type RxToken<'a> = JetonRx;
    type TxToken<'a> = JetonTx;

    fn receive(&mut self, _t: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let mut tampon = [0u8; TRAME_MAX];
        super::prends(&mut tampon).map(|n| (JetonRx { tampon, n }, JetonTx))
    }

    fn transmit(&mut self, _t: Instant) -> Option<Self::TxToken<'_>> {
        Some(JetonTx)
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut c = DeviceCapabilities::default();
        c.max_transmission_unit = 1500;
        c.medium = Medium::Ethernet;
        c
    }
}

fn maintenant() -> Instant {
    Instant::from_millis(crate::kernel::timer::monotonic_ms() as i64)
}

/// L'etat d'UNE connexion. Reinitialise a chaque acceptation.
struct Session {
    decoupeur: Decoupeur,
    /// Ce qui est arrive pendant qu'on repondait a la commande precedente.
    en_attente: FileCommandes,
    nonce: [u8; brdp::NONCE_LEN],
    authentifiee: bool,
    /// Curseur d'evenements pour `events tail` et `events watch`.
    curseur: u64,
    evenements: ModeEvenements,
    commandes: u64,
    /// Une commande a ete refusee faute de place : il faut le dire au client.
    file_debordee: bool,
    fermer: bool,
}

impl Session {
    fn neuve() -> Self {
        Self {
            decoupeur: Decoupeur::neuf(),
            en_attente: FileCommandes::neuve(),
            nonce: [0; brdp::NONCE_LEN],
            authentifiee: false,
            curseur: 0,
            evenements: ModeEvenements::Aucun,
            commandes: 0,
            file_debordee: false,
            fermer: false,
        }
    }

    /// Repart pour une nouvelle connexion, SANS reconstruire la file.
    ///
    /// `Session::neuve()` reconstruirait quatre kibioctets de tampons de
    /// commandes a chaque reconnexion, sur la pile d'un fil noyau. La remise a
    /// zero se fait donc en place, et `en_attente.purge()` garde au passage le
    /// compteur de refus -- c'est un compteur de service, pas un etat de
    /// connexion.
    fn recommence(&mut self) {
        self.decoupeur.reinitialise();
        self.en_attente.purge();
        self.nonce = [0; brdp::NONCE_LEN];
        self.authentifiee = false;
        self.curseur = 0;
        self.evenements = ModeEvenements::Aucun;
        self.commandes = 0;
        self.file_debordee = false;
        self.fermer = false;
    }

    /// Avale ce qui est arrive, et met CHAQUE ligne en file dans l'ordre.
    ///
    /// La regle -- et la raison pour laquelle la ligne trop longue est poussee
    /// a sa place dans le flux plutot qu'apres le decoupage -- est dans
    /// `session::avale`, qui est PUR et se contredit en test hote. Emprunts
    /// disjoints : `decoupeur` d'un cote, `en_attente` de l'autre, et
    /// `file_debordee` mis a jour une fois les deux rendus.
    fn avale(&mut self, octets: &[u8]) {
        self.file_debordee |= session::avale(&mut self.decoupeur, &mut self.en_attente, octets);
    }

    /// Un nonce par connexion : c'est ce qui interdit le rejeu direct.
    ///
    /// # Ce que ce nonce est, et ce qu'il n'est PAS
    ///
    /// Il n'est PAS cryptographiquement imprevisible, et ce module ne pretend
    /// pas le contraire : `rdtsc` melange a un compteur par un xorshift n'est
    /// pas un generateur aleatoire sur. Un attaquant qui connaitrait finement
    /// la frequence de la machine et l'instant de la connexion pourrait
    /// reduire l'espace des nonces.
    ///
    /// Ce qu'il apporte, et qui est ce dont on a besoin ici : l'UNICITE par
    /// connexion. Un HMAC intercepte sur une connexion precedente ne vaut rien
    /// sur la suivante, parce que le nonce a change. C'est le rejeu direct qui
    /// est ferme, et c'est la menace reelle sur un segment de laboratoire.
    ///
    /// Un vrai generateur reste a faire. Ce n'est pas le chantier du jour, et
    /// le dire vaut mieux que de laisser croire que c'en est un.
    fn tire_un_nonce(&mut self, connexion: u64) {
        let mut graine = crate::arch::x86_64::cpu::rdtsc() ^ connexion.wrapping_mul(0x9E37_79B9);
        for bloc in self.nonce.chunks_mut(8) {
            graine ^= graine << 13;
            graine ^= graine >> 7;
            graine ^= graine << 17;
            graine = graine.wrapping_add(crate::arch::x86_64::cpu::rdtsc());
            let octets = graine.to_le_bytes();
            let n = bloc.len();
            bloc.copy_from_slice(&octets[..n]);
        }
    }
}

/// L'annonce d'ouverture : version, nonce, algorithme.
fn annonce(t: &mut Reponse, nonce: &[u8]) {
    use core::fmt::Write;
    let mut hex = [0u8; brdp::NONCE_LEN * 2];
    brdp::hex_encode(nonce, &mut hex);
    t.vide();
    let _ = write!(
        t,
        "{{\"brdp\":{},\"auth\":\"hmac-sha256\",\"nonce\":\"{}\"}}",
        brdp::VERSION,
        core::str::from_utf8(&hex).unwrap_or(""),
    );
    t.termine();
}

/// Traite une ligne et rend la reponse. Ne touche PAS a la chaussette.
fn traite(session: &mut Session, ligne: &[u8], sortie: &mut Reponse) {
    // LA SENTINELLE EST TRAITEE ICI, a sa place dans la file. Voir
    // `Session::avale` : c'est ce qui garde l'ordre des reponses.
    if ligne == LIGNE_TROP_LONGUE {
        REFUSEES.fetch_add(1, Ordering::Relaxed);
        reponses::erreur(sortie, Erreur::TropLongue);
        return;
    }

    let commande = match brdp::analyse(ligne) {
        Ok(c) => c,
        Err(e) => {
            REFUSEES.fetch_add(1, Ordering::Relaxed);
            reponses::erreur(sortie, e);
            return;
        }
    };

    // L'ORDRE COMPTE : on verifie l'authentification AVANT d'executer, et non
    // apres. Une commande executee puis refusee aurait deja eu son effet.
    if !session.authentifiee && !commande.avant_auth() {
        REFUSEES.fetch_add(1, Ordering::Relaxed);
        reponses::erreur(sortie, Erreur::NonAuthentifie);
        return;
    }

    if let Commande::Hello { mac } = commande {
        let jeton = JETON.unwrap_or("").as_bytes();
        if brdp::verifie(jeton, &session.nonce, &mac) {
            session.authentifiee = true;
            AUTH_OK.fetch_add(1, Ordering::Relaxed);
            crate::kernel::lab::emets(
                crate::kernel::lab::Categorie::Remote,
                crate::kernel::lab::id::BRDP_AUTH,
                [1, 0, 0, 0],
            );
            reponses::rend(sortie, commande);
        } else {
            AUTH_KO.fetch_add(1, Ordering::Relaxed);
            crate::kernel::lab::emets(
                crate::kernel::lab::Categorie::Remote,
                crate::kernel::lab::id::BRDP_AUTH,
                [0, Erreur::AuthRefusee as u64, 0, 0],
            );
            reponses::erreur(sortie, Erreur::AuthRefusee);
            // UN HMAC FAUX FERME LA CONNEXION. Laisser reessayer sur le meme
            // nonce transformerait la fenetre d'authentification en oracle :
            // autant d'essais qu'on veut, sur un secret qui ne change pas.
            session.fermer = true;
        }
        return;
    }

    session.commandes += 1;
    COMMANDES.fetch_add(1, Ordering::Relaxed);
    let debut = crate::kernel::timer::monotonic_ns();

    match commande {
        Commande::EventsTail { combien } => {
            // UN ETAT, PAS UN BOOLEEN. La premiere redaction posait le curseur
            // puis mettait « suit les evenements » a FAUX : aucune routine ne
            // vidait donc jamais ce qu'on venait de demander, et `events tail
            // 100` ne rendait que son accuse de reception.
            session.curseur = crate::kernel::lab::queue(combien as usize);
            session.evenements = ModeEvenements::Queue { restant: combien };
        }
        Commande::EventsWatch => {
            session.curseur = crate::kernel::lab::queue(16);
            session.evenements = ModeEvenements::Suivi;
        }
        _ => {}
    }

    let continuer = reponses::rend(sortie, commande);
    if !continuer {
        session.fermer = true;
    }
    if sortie.tronque() {
        // MIEUX VAUT « JE N'AI PAS PU TOUT DIRE » QU'UNE LIGNE ILLISIBLE. Une
        // reponse coupee au milieu produit du JSON invalide, et le client
        // rejette la ligne entiere sans savoir pourquoi.
        reponses::erreur(sortie, Erreur::ArgumentInvalide);
    }
    crate::kernel::lab::emets(
        crate::kernel::lab::Categorie::Remote,
        crate::kernel::lab::id::BRDP_COMMANDE,
        [
            commande.code() as u64,
            sortie.len() as u64,
            crate::kernel::timer::monotonic_ns().saturating_sub(debut),
            0,
        ],
    );
}

/// Verse dans la chaussette ce qui tient, et pas un octet de plus.
///
/// # Pourquoi on n'attend PAS que la place se libere
///
/// Un client lent -- ou un client qui a cesse de lire sans fermer -- ferait
/// bloquer ce fil, et avec lui tout le canal d'enquete. `send_slice` prend ce
/// qui entre ; ce qui reste attendra le tour de boucle suivant, et si le
/// client ne lit jamais, c'est lui qui sera ferme par l'echeance, pas nous.
fn verse(sock: &mut tcp::Socket, octets: &[u8]) -> usize {
    if !sock.can_send() {
        return 0;
    }
    let n = sock.send_slice(octets).unwrap_or(0);
    OCTETS_RENDUS.fetch_add(n as u64, Ordering::Relaxed);
    n
}

fn fil_brdp() -> ! {
    let mut peripherique = Peripherique;
    let mac = super::mac();
    let mut config = Config::new(HardwareAddress::Ethernet(EthernetAddress(mac)));
    config.random_seed = crate::arch::x86_64::cpu::rdtsc();
    let mut iface = Interface::new(config, &mut peripherique, maintenant());
    let ip = super::ip();
    iface.update_ip_addrs(|ips| {
        let _ = ips.push(IpCidr::new(
            IpAddress::v4(ip[0], ip[1], ip[2], ip[3]),
            super::adresse::PREFIXE_BITS,
        ));
    });
    // PAS DE ROUTE PAR DEFAUT, ET PAS DE PASSERELLE. Ce n'est pas une
    // configuration reseau de repli : c'est un canal de diagnostic sur le
    // segment local. Une passerelle fictive ferait sortir du trafic vers une
    // adresse qui n'existe pas, et masquerait la vraie panne derriere des
    // delais d'attente.

    let mut chaussettes = SocketSet::new(Vec::new());
    let poignee = chaussettes.add(tcp::Socket::new(
        tcp::SocketBuffer::new(vec![0u8; 4096]),
        tcp::SocketBuffer::new(vec![0u8; 16384]),
    ));

    let mut session = Session::neuve();
    let mut ouverte = false;
    let mut sortie = Reponse::neuf();
    // Ce qui reste a verser quand la chaussette etait pleine.
    let mut reste: usize = 0;

    crate::kernel::lab::emets(
        crate::kernel::lab::Categorie::Remote,
        crate::kernel::lab::id::BRDP_ECOUTE,
        [super::PORT_BRDP as u64, 0, 0, 0],
    );

    loop {
        iface.poll(maintenant(), &mut peripherique, &mut chaussettes);
        let sock = chaussettes.get_mut::<tcp::Socket>(poignee);

        // Un FIN/RST distant peut laisser l'unique socket BRDP encore ouverte
        // cote smoltcp (ex. CLOSE-WAIT). `is_open()` seul ne suffit donc pas
        // pour recycler la session. Si plus rien ne peut etre recu, ferme
        // notre cote et laisse la boucle revenir ensuite vers LISTEN.
        if ouverte && !sock.may_recv() {
            FERMETURES_DISTANTES.fetch_add(1, Ordering::Relaxed);
            reste = 0;
            session.fermer = false;
            sock.close();
        }

        if !sock.is_open() {
            if ouverte {
                // La connexion precedente est finie.
                ouverte = false;
                crate::kernel::lab::emets(
                    crate::kernel::lab::Categorie::Remote,
                    crate::kernel::lab::id::BRDP_DECONNEXION,
                    [0, session.commandes, 0, 0],
                );
            }
            session.recommence();
            reste = 0;
            let _ = sock.listen(super::PORT_BRDP);
        }

        if sock.may_send() && !ouverte && sock.is_active() {
            // NOUVELLE CONNEXION. Le nonce est tire ici, une fois, et il ne
            // resservira jamais.
            ouverte = true;
            let n = CONNEXIONS.fetch_add(1, Ordering::Relaxed) + 1;
            session.tire_un_nonce(n);
            crate::kernel::lab::emets(
                crate::kernel::lab::Categorie::Remote,
                crate::kernel::lab::id::BRDP_CONNEXION,
                [0, 0, n, 0],
            );
            annonce(&mut sortie, &session.nonce);
            let pris = verse(sock, sortie.octets());
            reste = sortie.len() - pris;
        }

        // Le reliquat d'abord : l'ordre des lignes est le protocole.
        if reste > 0 {
            let deja = sortie.len() - reste;
            let pris = verse(sock, &sortie.octets()[deja..]);
            reste -= pris;
        }

        // 1. LIRE TOUT CE QUI EST ARRIVE, et le mettre en file.
        //
        // TCP est un flux : rien n'interdit au noyau distant de coller trois
        // commandes dans un segment, et c'est meme ce qu'il fait des que le
        // client enchaine. La premiere redaction n'en gardait qu'une et
        // perdait les suivantes, silencieusement. La file est BORNEE : un
        // tampon qui grandit avec ce qu'un pair envoie est une panne memoire
        // declenchable a distance.
        if ouverte && sock.can_recv() {
            let mut entree = [0u8; 1024];
            if let Ok(n) = sock.recv_slice(&mut entree) {
                session.avale(&entree[..n]);
            }
        }

        // 2. SERVIR UNE COMMANDE, et une seule : il n'y a qu'un tampon de
        //    reponse, et la reponse precedente doit etre entierement partie
        //    avant qu'on ecrase ce tampon.
        if ouverte && reste == 0 {
            let mut ligne = [0u8; brdp::LIGNE_MAX];
            if let Some(n) = session.en_attente.retire(&mut ligne) {
                traite(&mut session, &ligne[..n], &mut sortie);
                let pris = verse(sock, sortie.octets());
                reste = sortie.len() - pris;
            } else if session.file_debordee {
                // LE DEBORDEMENT SE DIT. Un client qui perd des commandes sans
                // le savoir attend des reponses qui ne viendront jamais, et ne
                // peut pas distinguer cela d'une machine figee.
                session.file_debordee = false;
                reponses::erreur(&mut sortie, Erreur::FilePleine);
                let pris = verse(sock, sortie.octets());
                reste = sortie.len() - pris;
            }
        }

        // 3. LE FLUX D'EVENEMENTS, quand rien d'autre n'attend d'etre ecrit.
        if ouverte
            && session.authentifiee
            && session.evenements.actif()
            && reste == 0
            && session.en_attente.est_vide()
        {
            reste = verse_des_evenements(sock, &mut session, &mut sortie);
        }

        if session.fermer && reste == 0 {
            sock.close();
            session.fermer = false;
        }

        // DIX MILLISECONDES. Assez pour que `events watch` soit vivant, assez
        // peu pour qu'un fil de diagnostic ne pese pas sur l'ordonnanceur
        // qu'on mesure par ailleurs.
        crate::kernel::task::sleep_ticks(crate::kernel::timer::ms_to_ticks(10));
    }
}

/// Verse au plus quelques evenements, et rend ce qui n'est pas passe.
fn verse_des_evenements(
    sock: &mut tcp::Socket,
    session: &mut Session,
    sortie: &mut Reponse,
) -> usize {
    use core::fmt::Write;
    // BORNE PAR TOUR. Un client qui demande le flux pendant qu'un auditeur a
    // dix hertz capture un bouclage complet recevrait sinon des centaines
    // d'evenements d'affilee, et ce fil ne rendrait plus la main.
    const PAR_TOUR: u32 = 16;

    // Le mode decide combien il en reste a servir : `Suivi` prend la borne du
    // tour, `Queue` prend le plus petit des deux. C'est ce qui fait qu'un
    // `events tail 5` en rend cinq et pas un de plus.
    let budget = session.evenements.budget(PAR_TOUR);
    let mut envoyes = 0u32;
    let mut reste = 0usize;
    let mut epuise = false;
    while envoyes < budget {
        if !sock.can_send() {
            break;
        }
        match crate::kernel::lab::lis(session.curseur) {
            crate::kernel::lab::Lecture::Evenement(ev) => {
                sortie.vide();
                let _ = crate::kernel::lab::catalogue::rend(
                    sortie,
                    &ev,
                    crate::kernel::lab::catalogue::Style::Json,
                );
                sortie.termine();
                let pris = verse(sock, sortie.octets());
                session.curseur += 1;
                envoyes += 1;
                if pris < sortie.len() {
                    reste = sortie.len() - pris;
                    break;
                }
            }
            crate::kernel::lab::Lecture::Ecrasee => {
                // LE TROU SE DIT, IL NE SE COMBLE PAS. Un client qui recevrait
                // une suite trouee presentee comme continue tirerait des
                // conclusions fausses sur la chronologie.
                let (recale_vers, _) = crate::kernel::lab::recale(session.curseur);
                // LA DESTINATION EST DECIDEE AVANT D'ETRE ANNONCEE, et le
                // curseur avance toujours d'au moins un : un recalage qui ne
                // ferait pas avancer relancerait le meme trou a chaque tour.
                // Voir `politique::LectureTransport`, qui porte la meme regle
                // pour le canal de telemetrie.
                //
                // `perdues` se DEDUIT de la destination au lieu d'etre repris
                // de `recale`. Les deux coincident des que le recalage avance
                // -- le cas courant -- mais pas quand la borne d'un cran joue :
                // annoncer alors le `0` de `recale` dirait au client qu'il n'a
                // rien manque, en sautant quand meme un evenement. Un trou tu
                // est precisement ce que cette branche existe pour empecher.
                let destination = recale_vers.max(session.curseur + 1);
                let perdues = destination - session.curseur;
                sortie.vide();
                let _ = write!(
                    sortie,
                    "{{\"lost\":{perdues},\"from\":{},\"to\":{destination}}}",
                    session.curseur,
                );
                sortie.termine();
                let pris = verse(sock, sortie.octets());
                session.curseur = destination;
                if pris < sortie.len() {
                    reste = sortie.len() - pris;
                }
                break;
            }
            crate::kernel::lab::Lecture::PasEncore
            | crate::kernel::lab::Lecture::Dechiree => {
                epuise = true;
                break;
            }
        }
    }

    session.evenements = session.evenements.consomme(envoyes);
    // UN `tail` QUI DEMANDE PLUS QUE L'ANNEAU N'A NE RESTE PAS PENDU.
    // `events tail 100` sur un anneau qui n'en contient que vingt en rend
    // vingt et se tait, au lieu d'attendre indefiniment les quatre-vingts qui
    // n'existent pas. `Suivi`, lui, attend : c'est son travail.
    if epuise && session.evenements.finit_sur_vide() {
        session.evenements = ModeEvenements::Aucun;
    }
    reste
}

/// Lance le serveur. Rend faux si aucun jeton n'a ete injecte au build.
pub fn demarre() -> bool {
    if LANCE.load(Ordering::Acquire) {
        return true;
    }
    if !super::actif() {
        return false;
    }
    let Some(jeton) = JETON else {
        // SANS JETON, RIEN N'ECOUTE. Un debugger en lecture seule sans
        // authentification reste un debugger qui publie l'etat interne de la
        // machine a quiconque atteint le segment local.
        crate::serial_println!(
            "BOUCHAUD_BRDP_DESACTIVE raison=sans-jeton \
indice=construire avec BOUCHAUD_DEBUG_TOKEN",
        );
        return false;
    };
    if jeton.is_empty() {
        crate::serial_println!("BOUCHAUD_BRDP_DESACTIVE raison=jeton-vide");
        return false;
    }
    if crate::kernel::task::spawn_noyau_priorite(
        fil_brdp,
        "bouchaud-brdp",
        crate::kernel::task::Priorite::Normale,
    ) {
        LANCE.store(true, Ordering::Release);
        let ip = super::ip();
        crate::serial_println!(
            "BOUCHAUD_BRDP_ECOUTE ip={}.{}.{}.{} port={}",
            ip[0], ip[1], ip[2], ip[3], super::PORT_BRDP,
        );
        return true;
    }
    false
}

pub fn lance() -> bool {
    LANCE.load(Ordering::Relaxed)
}

/// Le serveur est-il utilisable sur cette image ?
pub fn arme() -> bool {
    JETON.map(|j| !j.is_empty()).unwrap_or(false)
}

/// Connexions, authentifications reussies, refusees, commandes servies.
pub fn compteurs() -> (u64, u64, u64, u64) {
    (
        CONNEXIONS.load(Ordering::Relaxed),
        AUTH_OK.load(Ordering::Relaxed),
        AUTH_KO.load(Ordering::Relaxed),
        COMMANDES.load(Ordering::Relaxed),
    )
}

pub fn octets_rendus() -> u64 {
    OCTETS_RENDUS.load(Ordering::Relaxed)
}

pub fn refusees() -> u64 {
    REFUSEES.load(Ordering::Relaxed)
}
