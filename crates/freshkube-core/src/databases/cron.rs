//! Backup schedule expressions, read in the operator's own dialect:
//! CloudNativePG takes six fields (seconds first), the MariaDB operator five.
//! A shape this does not recognise stays the raw expression; nothing is guessed.
use super::Engine;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CronError {
    Empty,
    /// The expression has this many fields where the engine takes another.
    Fields {
        found: usize,
        expected: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Any,
    Every(u32),
    At(u32),
    /// Lists, ranges, names and the like: not described.
    Other,
}
impl Field {
    fn parse(text: &str, max: u32) -> Self {
        if text == "*" {
            return Self::Any;
        }
        if let Some(step) = text.strip_prefix("*/") {
            return match step.parse::<u32>() {
                Ok(step) if step > 0 && step <= max => Self::Every(step),
                _ => Self::Other,
            };
        }
        match text.parse::<u32>() {
            Ok(value) if value <= max => Self::At(value),
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cron {
    raw: String,
    seconds: Field,
    minute: Field,
    hour: Field,
    day: Field,
    month: Field,
    weekday: Field,
}

impl Cron {
    pub fn parse(engine: Engine, expression: &str) -> Result<Self, CronError> {
        let expression = expression.trim();
        if expression.is_empty() {
            return Err(CronError::Empty);
        }
        let expected = match engine {
            Engine::CloudNativePg => 6,
            Engine::MariaDb => 5,
        };
        if let Some(macro_) = expression.strip_prefix('@') {
            let fields = match macro_ {
                "hourly" => "0 0 * * * *",
                "daily" | "midnight" => "0 0 0 * * *",
                "weekly" => "0 0 0 * * 0",
                "monthly" => "0 0 0 1 * *",
                "yearly" | "annually" => "0 0 0 1 1 *",
                _ => {
                    return Ok(Self::other(expression));
                }
            };
            let mut cron = Self::parse(Engine::CloudNativePg, fields)?;
            cron.raw = expression.to_owned();
            return Ok(cron);
        }
        let parts: Vec<&str> = expression.split_whitespace().collect();
        if parts.len() != expected {
            return Err(CronError::Fields {
                found: parts.len(),
                expected,
            });
        }
        // MariaDB's five fields have no seconds: they fire at second zero.
        let (seconds, rest) = if expected == 6 {
            (Field::parse(parts[0], 59), &parts[1..])
        } else {
            (Field::At(0), &parts[..])
        };
        Ok(Self {
            raw: expression.to_owned(),
            seconds,
            minute: Field::parse(rest[0], 59),
            hour: Field::parse(rest[1], 23),
            day: Field::parse(rest[2], 31),
            month: Field::parse(rest[3], 12),
            weekday: Field::parse(rest[4], 7),
        })
    }

    fn other(expression: &str) -> Self {
        Self {
            raw: expression.to_owned(),
            seconds: Field::Other,
            minute: Field::Other,
            hour: Field::Other,
            day: Field::Other,
            month: Field::Other,
            weekday: Field::Other,
        }
    }

    /// The expression as written.
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Plain words for the shapes backups use, or none.
    pub fn describe(&self) -> Option<String> {
        use Field::*;
        if self.seconds != At(0) || self.month != Any {
            return None;
        }
        let time = |hour: u32, minute: u32| format!("{hour:02}:{minute:02}");
        match (self.minute, self.hour, self.day, self.weekday) {
            (Any, Any, Any, Any) => Some("Every minute".into()),
            (Every(n), Any, Any, Any) => Some(format!("Every {n} minutes")),
            (At(m), Any, Any, Any) => Some(format!("Hourly at :{m:02}")),
            (At(0), Every(n), Any, Any) => Some(format!("Every {n} hours")),
            (At(m), Every(n), Any, Any) => Some(format!("Every {n} hours at :{m:02}")),
            (At(m), At(h), Any, Any) => Some(format!("Daily at {}", time(h, m))),
            (At(m), At(h), Any, At(d)) => {
                Some(format!("Weekly on {} at {}", weekday(d)?, time(h, m)))
            }
            (At(m), At(h), At(day), Any) => Some(format!("Monthly on day {day} at {}", time(h, m))),
            _ => None,
        }
    }

    /// The words, else the expression itself.
    pub fn words(&self) -> String {
        self.describe().unwrap_or_else(|| self.raw.clone())
    }
}

fn weekday(day: u32) -> Option<&'static str> {
    Some(match day {
        0 | 7 => "Sunday",
        1 => "Monday",
        2 => "Tuesday",
        3 => "Wednesday",
        4 => "Thursday",
        5 => "Friday",
        6 => "Saturday",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(engine: Engine, text: &str) -> String {
        Cron::parse(engine, text).unwrap().words()
    }

    #[test]
    fn cloudnativepg_takes_six_fields_seconds_first() {
        assert_eq!(
            words(Engine::CloudNativePg, "0 0 2 * * *"),
            "Daily at 02:00"
        );
        assert_eq!(
            words(Engine::CloudNativePg, "0 30 3 * * 0"),
            "Weekly on Sunday at 03:30"
        );
        assert_eq!(
            words(Engine::CloudNativePg, "0 0 */6 * * *"),
            "Every 6 hours"
        );
        assert_eq!(
            words(Engine::CloudNativePg, "0 0 1 1 * *"),
            "Monthly on day 1 at 01:00"
        );
    }

    #[test]
    fn mariadb_takes_five_fields() {
        assert_eq!(words(Engine::MariaDb, "0 2 * * *"), "Daily at 02:00");
        assert_eq!(words(Engine::MariaDb, "*/15 * * * *"), "Every 15 minutes");
        assert_eq!(words(Engine::MariaDb, "0 */6 * * *"), "Every 6 hours");
        assert_eq!(words(Engine::MariaDb, "15 * * * *"), "Hourly at :15");
        assert_eq!(
            words(Engine::MariaDb, "0 4 * * 1"),
            "Weekly on Monday at 04:00"
        );
    }

    #[test]
    fn the_wrong_field_count_is_refused_not_reinterpreted() {
        assert_eq!(
            Cron::parse(Engine::CloudNativePg, "0 2 * * *"),
            Err(CronError::Fields {
                found: 5,
                expected: 6
            })
        );
        assert_eq!(
            Cron::parse(Engine::MariaDb, "0 0 2 * * *"),
            Err(CronError::Fields {
                found: 6,
                expected: 5
            })
        );
        assert_eq!(Cron::parse(Engine::MariaDb, "  "), Err(CronError::Empty));
    }

    #[test]
    fn shapes_it_does_not_know_stay_raw() {
        assert_eq!(words(Engine::MariaDb, "0 2,14 * * *"), "0 2,14 * * *");
        assert_eq!(words(Engine::MariaDb, "0 2 * 6 *"), "0 2 * 6 *");
        assert_eq!(words(Engine::MariaDb, "0 99 * * *"), "0 99 * * *");
        assert_eq!(words(Engine::CloudNativePg, "30 0 2 * * *"), "30 0 2 * * *");
        assert_eq!(words(Engine::MariaDb, "0 2 * * MON"), "0 2 * * MON");
    }

    #[test]
    fn macros_describe_their_shape_and_unknown_ones_stay_raw() {
        let daily = Cron::parse(Engine::CloudNativePg, "@daily").unwrap();
        assert_eq!(daily.describe().as_deref(), Some("Daily at 00:00"));
        assert_eq!(daily.raw(), "@daily");
        assert_eq!(words(Engine::MariaDb, "@every 1h"), "@every 1h");
    }
}
