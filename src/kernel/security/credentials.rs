use super::capability::Capabilities;

pub const MAX_GROUPS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Credentials {
    pub ruid: u32,
    pub euid: u32,
    pub suid: u32,
    pub rgid: u32,
    pub egid: u32,
    pub sgid: u32,
    pub groups: [u32; MAX_GROUPS],
    pub group_count: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialError {
    PermissionDenied,
}

impl Credentials {
    pub const fn new(uid: u32, gid: u32) -> Self {
        Self {
            ruid: uid,
            euid: uid,
            suid: uid,
            rgid: gid,
            egid: gid,
            sgid: gid,
            groups: [0; MAX_GROUPS],
            group_count: 0,
        }
    }

    pub fn set_uid(
        &mut self,
        target: u32,
        caps: Capabilities,
    ) -> Result<(), CredentialError> {
        if caps.contains(Capabilities::SET_IDENTITY) {
            self.ruid = target;
            self.euid = target;
            self.suid = target;
            return Ok(());
        }

        if target == self.ruid || target == self.suid {
            self.euid = target;
            Ok(())
        } else {
            Err(CredentialError::PermissionDenied)
        }
    }

    pub fn set_gid(
        &mut self,
        target: u32,
        caps: Capabilities,
    ) -> Result<(), CredentialError> {
        if caps.contains(Capabilities::SET_IDENTITY) {
            self.rgid = target;
            self.egid = target;
            self.sgid = target;
            return Ok(());
        }

        if target == self.rgid || target == self.sgid {
            self.egid = target;
            Ok(())
        } else {
            Err(CredentialError::PermissionDenied)
        }
    }

    pub fn in_group(&self, gid: u32) -> bool {
        if self.egid == gid {
            return true;
        }
        self.groups[..self.group_count as usize].contains(&gid)
    }
}
