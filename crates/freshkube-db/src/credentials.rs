//! How a connection's password is held. Which Secret it comes from is
//! chosen where the cluster is read (`freshkube-core`'s `databases::explore`:
//! the team's reader Secret, then the operator's app user, never the
//! superuser). The caller reads that Secret once, when it connects, and puts
//! the password in a [`Password`], which is only in memory, wiped when
//! dropped, and prints as `<redacted>`.
use std::fmt;
use zeroize::Zeroizing;

pub struct Password(Zeroizing<String>);

impl Password {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }

    /// For the connection's startup message only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(<redacted>)")
    }
}

impl fmt::Display for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_never_prints() {
        let p = Password::new("hunter2");
        assert!(!format!("{p:?} {p}").contains("hunter2"));
        assert_eq!(p.expose(), "hunter2");
    }
}
