// Types publics historiques.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
pub enum Source {
    Clavier = 0,
    Souris = 1,
    Client = 2,
    Fenetre = 3,
    Explicite = 4,
}

pub const NOMBRE_SOURCES: usize = 5;
pub const NOMS_SOURCES: [&str; NOMBRE_SOURCES] =
    ["clavier", "souris", "client", "fenetre", "explicite"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fin {
    DejaSignale,
    Signale,
    Echeance,
}

#[derive(Clone, Copy)]
pub struct Billet {
    source: WaitSourceTicket,
}
