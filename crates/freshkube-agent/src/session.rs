//! A session's identity and its secret.
//!
//! The token is 32 bytes of OS randomness. It reaches the agent once, inside
//! ACP `session/new` over stdin, and never through argv, the environment, a
//! file, a log or `Debug`: its `Debug` and `Display` print nothing of it, and
//! the one way to read it is [`Token::reveal`].

use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};

/// A session, as a random UUID the app makes.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(uuid::Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    /// The first eight characters, which is what a log line names.
    pub fn short(&self) -> String {
        self.0.simple().to_string()[..8].to_owned()
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(f)
    }
}

impl fmt::Debug for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SessionId({})", self.short())
    }
}

/// A read's id within its session: `r1`, `r2`, …. The agent cites it; the
/// app's trail and follower use it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ReadId(u32);

impl ReadId {
    /// The id with this number, for an event a test makes by hand; the
    /// endpoint hands out its own in order.
    pub fn numbered(number: u32) -> Self {
        Self(number)
    }

    pub fn number(self) -> u32 {
        self.0
    }
}

impl fmt::Display for ReadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r{}", self.0)
    }
}

/// Hands out a session's read ids in order.
#[derive(Default)]
pub(crate) struct ReadIds(AtomicU32);

impl ReadIds {
    pub(crate) fn next(&self) -> ReadId {
        ReadId(self.0.fetch_add(1, Ordering::SeqCst) + 1)
    }
}

/// The session's bearer token.
#[derive(Clone)]
pub struct Token(String);

impl Token {
    /// 32 bytes from the OS, as 64 hex digits.
    pub fn generate() -> std::io::Result<Self> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
        Ok(Self(
            bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        ))
    }

    /// The token, for the one place that hands it to the agent.
    pub fn reveal(&self) -> &str {
        &self.0
    }

    /// Whether `header` is `Bearer <this token>`, in time that does not
    /// depend on how much of it matches.
    pub(crate) fn accepts(&self, header: &[u8]) -> bool {
        const SCHEME: &[u8] = b"Bearer ";
        let token = self.0.as_bytes();
        let shape = header.len() == SCHEME.len() + token.len();
        // The comparison runs over the longer length even when the shape is
        // wrong, so a wrong length and a wrong byte take the same time.
        let mut difference = u8::from(!shape);
        for at in 0..(SCHEME.len() + token.len()) {
            let want = if at < SCHEME.len() {
                SCHEME[at]
            } else {
                token[at - SCHEME.len()]
            };
            difference |= want ^ header.get(at).copied().unwrap_or(0);
        }
        difference == 0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(..)")
    }
}

impl fmt::Display for Token {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_random_and_accepts_only_its_own_bearer_header() {
        let (a, b) = (Token::generate().unwrap(), Token::generate().unwrap());
        assert_eq!(a.reveal().len(), 64);
        assert_ne!(a.reveal(), b.reveal());
        assert!(a.accepts(format!("Bearer {}", a.reveal()).as_bytes()));
        assert!(!a.accepts(format!("Bearer {}", b.reveal()).as_bytes()));
        assert!(!a.accepts(a.reveal().as_bytes()));
        assert!(!a.accepts(b""));
        assert!(!a.accepts(format!("bearer {}", a.reveal()).as_bytes()));
        assert!(!a.accepts(format!("Bearer {}x", a.reveal()).as_bytes()));
    }

    #[test]
    fn read_ids_count_from_one() {
        let ids = ReadIds::default();
        assert_eq!(ids.next().to_string(), "r1");
        assert_eq!(ids.next().to_string(), "r2");
    }
}
