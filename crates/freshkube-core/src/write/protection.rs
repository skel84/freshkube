use serde::{Deserialize, Serialize};

/// The reason a control gives when a `read-only` cluster refuses a write.
pub const READ_ONLY_REASON: &str = "This cluster is read-only.";

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
    /// Every write asks the user to type the object's name.
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
            Protection::Protected => "Every write asks you to type the object's name.",
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
