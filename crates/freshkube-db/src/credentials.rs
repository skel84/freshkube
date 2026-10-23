//! Which credential a connection uses, and how the password is held.
//!
//! The team-named reader Secret comes first; the operator's app user is next
//! (its role can write, its session can't). The superuser Secret is never
//! offered: [`choose`] has no way to return one. The caller reads the chosen
//! Secret once, when it connects, and puts the password in a [`Password`],
//! which is only in memory, wiped when dropped, and prints as `<redacted>`.
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

/// A Secret the caller found, by name. No value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub secret: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The Secret the team names for read-only access.
    Reader,
    /// The operator's application user.
    AppUser,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub kind: Kind,
    pub secret: String,
}

/// The first of the reader Secret and the app-user Secret that exists.
/// `None` is the "no reader Secret" state: the connect step says what to do.
pub fn choose(reader: Option<Offer>, app_user: Option<Offer>) -> Option<Choice> {
    reader
        .map(|o| Choice {
            kind: Kind::Reader,
            secret: o.secret,
        })
        .or_else(|| {
            app_user.map(|o| Choice {
                kind: Kind::AppUser,
                secret: o.secret,
            })
        })
}

/// The Secret name CloudNativePG gives a cluster's application user.
pub fn cnpg_app_secret(cluster: &str) -> String {
    format!("{cluster}-app")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(name: &str) -> Option<Offer> {
        Some(Offer {
            secret: name.into(),
        })
    }

    #[test]
    fn the_reader_comes_before_the_app_user() {
        let c = choose(offer("orders-reader"), offer("orders-db-app")).unwrap();
        assert_eq!((c.kind, c.secret.as_str()), (Kind::Reader, "orders-reader"));
    }

    #[test]
    fn the_app_user_is_next_and_nothing_is_nothing() {
        let c = choose(None, offer("orders-db-app")).unwrap();
        assert_eq!(c.kind, Kind::AppUser);
        assert_eq!(choose(None, None), None);
    }

    #[test]
    fn the_app_secret_is_not_the_superuser_secret() {
        assert_eq!(cnpg_app_secret("orders-db"), "orders-db-app");
        assert!(!cnpg_app_secret("orders-db").contains("superuser"));
    }

    #[test]
    fn a_password_never_prints() {
        let p = Password::new("hunter2");
        assert!(!format!("{p:?} {p}").contains("hunter2"));
        assert_eq!(p.expose(), "hunter2");
    }
}
