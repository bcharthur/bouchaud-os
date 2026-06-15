//! Modele utilisateur et session courante.
//!
//! Modele volontairement simple (root / arthur / guest) sans mots de passe ni
//! isolation. Il prepare les notions futures de syscalls, processus et
//! permissions completes une fois le split user/kernel en place.

#[derive(Copy, Clone, PartialEq)]
pub enum User {
    Root,
    Arthur,
    Guest,
}

pub struct Session {
    current: User,
}

static mut SESSION: Session = Session { current: User::Root };

impl Session {
    pub fn login(&mut self, user: User) {
        self.current = user;
    }

    pub fn user(&self) -> User {
        self.current
    }

    pub fn username(&self) -> &'static str {
        match self.current {
            User::Root => "root",
            User::Arthur => "arthur",
            User::Guest => "guest",
        }
    }

    pub fn uid(&self) -> u16 {
        match self.current {
            User::Root => 0,
            User::Arthur => 1000,
            User::Guest => 65534,
        }
    }

    pub fn gid(&self) -> u16 {
        self.uid()
    }

    pub fn is_root(&self) -> bool {
        self.current == User::Root
    }
}

/// Accede a la session globale courante.
pub fn session() -> &'static mut Session {
    unsafe { &mut SESSION }
}

/// Resout un nom d'utilisateur connu.
pub fn user_from_name(name: &str) -> Option<User> {
    match name {
        "root" => Some(User::Root),
        "arthur" => Some(User::Arthur),
        "guest" => Some(User::Guest),
        _ => None,
    }
}
