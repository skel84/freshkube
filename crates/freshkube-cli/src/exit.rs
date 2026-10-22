//! The exit codes, and the order they win in when several apply.

/// Everything a run found, from which the exit code follows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Findings {
    /// A check failed, or a read failed outright.
    pub failed: bool,
    /// A read was refused (403).
    pub refused: bool,
    /// A sign-in is needed.
    pub sign_in: bool,
    /// Something could not be told (unreachable, timed out).
    pub unknown: bool,
}

pub const OK: i32 = 0;
pub const FAILED: i32 = 1;
pub const USAGE: i32 = 2;
pub const UNKNOWN: i32 = 3;
pub const REFUSED: i32 = 4;
pub const SIGN_IN: i32 = 5;

impl Findings {
    /// Usage (2) is decided before anything runs and never reaches here.
    /// Then anything failed gives 1, else anything refused 4, else a needed
    /// sign-in 5, else anything unknown 3, else 0.
    pub fn code(self) -> i32 {
        if self.failed {
            FAILED
        } else if self.refused {
            REFUSED
        } else if self.sign_in {
            SIGN_IN
        } else if self.unknown {
            UNKNOWN
        } else {
            OK
        }
    }

    pub fn merge(&mut self, other: Findings) {
        self.failed |= other.failed;
        self.refused |= other.refused;
        self.sign_in |= other.sign_in;
        self.unknown |= other.unknown;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_finding_alone_has_its_code() {
        let only = |f: fn(&mut Findings)| {
            let mut findings = Findings::default();
            f(&mut findings);
            findings.code()
        };
        assert_eq!(Findings::default().code(), 0);
        assert_eq!(only(|f| f.failed = true), 1);
        assert_eq!(only(|f| f.unknown = true), 3);
        assert_eq!(only(|f| f.refused = true), 4);
        assert_eq!(only(|f| f.sign_in = true), 5);
    }

    #[test]
    fn when_several_apply_the_fixed_order_decides() {
        let all = Findings {
            failed: true,
            refused: true,
            sign_in: true,
            unknown: true,
        };
        assert_eq!(all.code(), 1);
        assert_eq!(
            Findings {
                failed: false,
                ..all
            }
            .code(),
            4
        );
        assert_eq!(
            Findings {
                failed: false,
                refused: false,
                ..all
            }
            .code(),
            5
        );
        assert_eq!(
            Findings {
                failed: false,
                refused: false,
                sign_in: false,
                ..all
            }
            .code(),
            3
        );
    }
}
