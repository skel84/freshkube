//! A row's state: the worse of what the operator reports and what Coroot
//! judges, with the source that decided it.
use crate::HealthIndicator;

/// Who supplied a state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Operator,
    Coroot,
    /// Both give the same, non-Unknown answer.
    Both,
}
impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Self::Operator => "Operator",
            Self::Coroot => "Coroot",
            Self::Both => "Both",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowState {
    pub health: HealthIndicator,
    pub source: Source,
}

/// How bad a state is; Unknown ranks lowest so it never worsens a row.
pub(super) fn severity(health: HealthIndicator) -> u8 {
    match health {
        HealthIndicator::Error => 4,
        HealthIndicator::Warning => 3,
        HealthIndicator::Pending => 2,
        HealthIndicator::Healthy | HealthIndicator::Info => 1,
        HealthIndicator::Unknown => 0,
    }
}

/// The worse of the operator's answer and Coroot's. A Coroot Unknown (also
/// what an absent, missing or stale Coroot gives) leaves the operator's
/// answer alone; an operator Unknown leaves Coroot's.
pub fn worse_of(operator: HealthIndicator, coroot: HealthIndicator) -> RowState {
    let (o, c) = (severity(operator), severity(coroot));
    if c == 0 {
        RowState {
            health: operator,
            source: Source::Operator,
        }
    } else if o == 0 {
        RowState {
            health: coroot,
            source: Source::Coroot,
        }
    } else if c > o {
        RowState {
            health: coroot,
            source: Source::Coroot,
        }
    } else if c < o {
        RowState {
            health: operator,
            source: Source::Operator,
        }
    } else {
        RowState {
            health: operator,
            source: Source::Both,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use HealthIndicator::*;

    #[test]
    fn a_coroot_unknown_never_worsens_a_row() {
        for operator in [Healthy, Warning, Error, Unknown] {
            let state = worse_of(operator, Unknown);
            assert_eq!(state.health, operator);
            assert_eq!(state.source, Source::Operator);
        }
    }

    #[test]
    fn the_worse_answer_wins_and_names_its_source() {
        assert_eq!(
            worse_of(Healthy, Warning),
            RowState {
                health: Warning,
                source: Source::Coroot
            }
        );
        assert_eq!(
            worse_of(Error, Warning),
            RowState {
                health: Error,
                source: Source::Operator
            }
        );
        assert_eq!(
            worse_of(Warning, Warning),
            RowState {
                health: Warning,
                source: Source::Both
            }
        );
        assert_eq!(worse_of(Healthy, Healthy).source, Source::Both);
    }

    #[test]
    fn an_operator_unknown_takes_coroots_known_answer() {
        assert_eq!(
            worse_of(Unknown, Healthy),
            RowState {
                health: Healthy,
                source: Source::Coroot
            }
        );
        assert_eq!(worse_of(Unknown, Unknown).health, Unknown);
    }
}
