use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use serde::{Deserialize, Serialize};

/// The reason a control gives when a `read-only` cluster refuses a write.
pub const READ_ONLY_REASON: &str = "This cluster is read-only.";

/// The reason a dock shell gives when its own cluster's entry is no longer
/// listed.
pub const ENTRY_GONE_REASON: &str = "Its cluster entry is no longer in freshkube.yaml.";

/// The reason a dock shell gives while its cluster's level is not known: the
/// setup has not been read yet, so nothing says what the level is.
pub const LEVEL_UNKNOWN_REASON: &str = "The setup hasn't been read yet.";

/// What a dock tab knows of the level of its own cluster. It is the tab's
/// own cluster's, never the shown one's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnLevel {
    /// The level of the tab's entry, or of its context for a window with no
    /// entry, under the setup as it is now.
    Known(Protection),
    /// The entry the tab started under is no longer in the setup.
    EntryGone,
    /// The setup has not been read, so there is no level to apply.
    Unknown,
}

/// Whether a dock shell may start a session, from its own cluster's level
/// and nothing else. The shown cluster's level is not a parameter, so a call
/// site cannot pass it. A level that is gone or unknown blocks, as a skipped
/// entry stays read-only. `Err` is the reason the Start control gives.
pub fn may_start_shell(own: OwnLevel) -> Result<(), &'static str> {
    match own {
        OwnLevel::EntryGone => Err(ENTRY_GONE_REASON),
        OwnLevel::Unknown => Err(LEVEL_UNKNOWN_REASON),
        OwnLevel::Known(level) => level.refusal().map_or(Ok(()), Err),
    }
}

/// How much the app asks before it writes to a cluster. A person sets it on a
/// cluster entry; it is never inferred from a context's name, an id or a
/// role. The order is from the strictest to the loosest.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum Protection {
    /// No writes at all.
    ReadOnly,
    /// Every write asks the user to type the object's name, except one that
    /// only adds an annotation (it declares `no_confirmation`).
    Protected,
    /// Each action's own confirmation: the level of an entry that sets none.
    #[default]
    Guarded,
    /// Reversible actions skip their confirmation.
    Open,
}

impl Protection {
    pub const ALL: [Protection; 4] = [
        Protection::ReadOnly,
        Protection::Protected,
        Protection::Guarded,
        Protection::Open,
    ];

    /// The word the file spells it with.
    pub fn key(self) -> &'static str {
        match self {
            Protection::ReadOnly => "read-only",
            Protection::Protected => "protected",
            Protection::Guarded => "guarded",
            Protection::Open => "open",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Protection::ReadOnly => "Read-only",
            Protection::Protected => "Protected",
            Protection::Guarded => "Guarded",
            Protection::Open => "Open",
        }
    }

    /// What the level means, in a sentence for a tooltip or a hint.
    pub fn meaning(self) -> &'static str {
        match self {
            Protection::ReadOnly => "No writes at all.",
            Protection::Protected => {
                "Every write asks you to type the object's name, except one that only adds an annotation (Refresh, Reverify)."
            }
            Protection::Guarded => "Each action asks for its own confirmation.",
            Protection::Open => "Reversible actions skip their confirmation.",
        }
    }

    /// Whether this level refuses every write.
    pub fn is_read_only(self) -> bool {
        self == Protection::ReadOnly
    }

    /// Why a write is refused, when this level refuses it: what a disabled
    /// control says.
    pub fn refusal(self) -> Option<&'static str> {
        self.is_read_only().then_some(READ_ONLY_REASON)
    }

    /// The stricter of two levels.
    pub fn stricter(self, other: Protection) -> Protection {
        self.min(other)
    }
}

/// The level of one cluster as it is now, shared with a runner on another
/// thread, so a write reads it again just before it is sent: a level that
/// got stricter while a dialog was open is not missed.
#[derive(Clone, Debug)]
pub struct LevelCell(Arc<AtomicU8>);

impl LevelCell {
    pub fn new(level: Protection) -> Self {
        Self(Arc::new(AtomicU8::new(Self::code(level))))
    }

    fn code(level: Protection) -> u8 {
        Protection::ALL
            .iter()
            .position(|candidate| *candidate == level)
            .unwrap_or(0) as u8
    }

    pub fn get(&self) -> Protection {
        Protection::ALL
            .get(self.0.load(Ordering::SeqCst) as usize)
            .copied()
            // An unreadable code is the strictest level, never a looser one.
            .unwrap_or(Protection::ReadOnly)
    }

    pub fn set(&self, level: Protection) {
        self.0.store(Self::code(level), Ordering::SeqCst);
    }
}

/// Two cells are equal when they hold the same level now.
impl PartialEq for LevelCell {
    fn eq(&self, other: &Self) -> bool {
        self.get() == other.get()
    }
}

impl Eq for LevelCell {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shell_starts_from_its_own_level_at_each_level_and_never_without_one() {
        assert_eq!(
            may_start_shell(OwnLevel::Known(Protection::ReadOnly)),
            Err(READ_ONLY_REASON)
        );
        for level in [Protection::Protected, Protection::Guarded, Protection::Open] {
            assert_eq!(may_start_shell(OwnLevel::Known(level)), Ok(()));
        }
        assert_eq!(may_start_shell(OwnLevel::EntryGone), Err(ENTRY_GONE_REASON));
        assert_eq!(
            may_start_shell(OwnLevel::Unknown),
            Err(LEVEL_UNKNOWN_REASON)
        );
        assert_ne!(ENTRY_GONE_REASON, LEVEL_UNKNOWN_REASON);
    }

    #[test]
    fn the_default_is_guarded() {
        assert_eq!(Protection::default(), Protection::Guarded);
    }

    #[test]
    fn the_file_spells_each_level_as_its_key() {
        for level in Protection::ALL {
            let text = serde_json::to_string(&level).unwrap();
            assert_eq!(text, format!("\"{}\"", level.key()));
            assert_eq!(serde_json::from_str::<Protection>(&text).unwrap(), level);
        }
    }

    #[test]
    fn only_the_four_exact_words_are_read() {
        for text in [
            "Protected",
            "READ-ONLY",
            "readonly",
            "read_only",
            "",
            "prod",
        ] {
            assert!(
                serde_json::from_str::<Protection>(&format!("\"{text}\"")).is_err(),
                "{text}"
            );
        }
        assert!(serde_json::from_str::<Protection>("1").is_err());
    }

    #[test]
    fn the_stricter_level_wins() {
        use Protection::*;
        assert_eq!(Open.stricter(Guarded), Guarded);
        assert_eq!(Guarded.stricter(Protected), Protected);
        assert_eq!(Protected.stricter(ReadOnly), ReadOnly);
        assert_eq!(ReadOnly.stricter(Open), ReadOnly);
        assert_eq!(Open.stricter(Open), Open);
    }

    #[test]
    fn only_read_only_refuses_every_write() {
        for level in Protection::ALL {
            assert_eq!(level.is_read_only(), level == Protection::ReadOnly);
            assert_eq!(
                level.refusal(),
                level.is_read_only().then_some("This cluster is read-only.")
            );
        }
    }

    #[test]
    fn the_level_cell_holds_each_level_and_is_shared() {
        let cell = LevelCell::new(Protection::Guarded);
        let other = cell.clone();
        for level in Protection::ALL {
            other.set(level);
            assert_eq!(cell.get(), level);
        }
    }
}
