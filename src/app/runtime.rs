//! Types d'applications natives connues du systeme.

/// Application native identifiee par son `exec`.
#[derive(Clone, Copy, PartialEq)]
pub enum AppKind {
    Terminal,
    Files,
    Browser,
    Monitor,
    Unknown,
}

impl AppKind {
    /// Resout le champ `exec` d'un manifeste en application native.
    pub fn from_exec(exec: &str) -> AppKind {
        match exec {
            "terminal" => AppKind::Terminal,
            "files" => AppKind::Files,
            "browser" | "chromium" => AppKind::Browser,
            "monitor" | "sysinfo" => AppKind::Monitor,
            _ => AppKind::Unknown,
        }
    }

    pub fn is_gui(self) -> bool {
        !matches!(self, AppKind::Unknown)
    }
}
