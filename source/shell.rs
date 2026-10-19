use std::ffi::OsStr;

use crate::BoxError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Shell {
    Bash,
    Zsh,
    Fish,
}

impl Shell {
    pub(crate) fn parse(value: &OsStr) -> Result<Self, BoxError> {
        match value.to_str() {
            Some("bash") => Ok(Self::Bash),
            Some("zsh") => Ok(Self::Zsh),
            Some("fish") => Ok(Self::Fish),
            _ => Err(format!(
                "unsupported shell: {}; choose bash, zsh, or fish",
                value.to_string_lossy()
            )
            .into()),
        }
    }

    pub(crate) fn script(self) -> &'static str {
        match self {
            Self::Bash => include_str!("shell/bash.bash"),
            Self::Zsh => include_str!("shell/zsh.zsh"),
            Self::Fish => include_str!("shell/fish.fish"),
        }
    }
}
