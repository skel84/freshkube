//! What an agent sends, parsed into closed, bounded types. `Call::parse` is
//! the only place agent input is read: a tool and its arguments in, a
//! `Call` or a refusal out. Nothing past it holds a string the agent chose
//! that has not been checked for its alphabet and length.

use serde_json::Value;

use super::refusal::Refusal;
use crate::constants::agent::{
    DEFAULT_EVENT_SINCE_SECONDS, DEFAULT_ITEMS, DEFAULT_LOG_LINES, MAX_ENTRY_ID_BYTES, MAX_ITEMS,
    MAX_LOG_LINES, MAX_SINCE_SECONDS,
};

/// The four tools, in the order the endpoint lists them.
pub const TOOLS: [&str; 4] = ["list_objects", "get_object", "pod_logs", "list_events"];

/// The MCP server's name.
pub const SERVER_NAME: &str = "freshkube";

macro_rules! text {
    ($(#[$meta:meta])* $name:ident, $max:expr, $check:expr) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            fn parse(argument: &str, value: &str) -> Result<Self, Refusal> {
                let check: fn(&str) -> bool = $check;
                if value.is_empty() {
                    return Err(Refusal::invalid(argument, "is empty"));
                }
                if value.len() > $max {
                    return Err(Refusal::invalid(argument, "is too long"));
                }
                if !check(value) {
                    return Err(Refusal::invalid(argument, "has characters it may not have"));
                }
                Ok(Self(value.to_owned()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

fn ends_alphanumeric(bytes: &[u8]) -> bool {
    bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
}

fn dns_label(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && ends_alphanumeric(value.as_bytes())
}

fn dns_subdomain(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
        && ends_alphanumeric(value.as_bytes())
}

text!(
    /// A cluster entry's id.
    EntryId, MAX_ENTRY_ID_BYTES, |v| v.bytes().all(|b| b.is_ascii_graphic())
);
text!(
    /// What kubectl takes for a kind: `pods`, `Deployment`, `deploy.apps`.
    KindName, 253, |v| v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && ends_alphanumeric(v.as_bytes())
);
text!(
    /// A namespace: a DNS label.
    Namespace, 63, dns_label
);
text!(
    /// An object's name: a DNS subdomain.
    Name, 253, dns_subdomain
);
text!(
    /// A container's name: a DNS label.
    Container, 63, dns_label
);

/// `list_objects`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct List {
    pub kind: KindName,
    pub namespace: Option<Namespace>,
    pub limit: usize,
}

/// `get_object`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Get {
    pub kind: KindName,
    pub namespace: Option<Namespace>,
    pub name: Name,
}

/// `pod_logs`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Logs {
    pub namespace: Namespace,
    pub pod: Name,
    pub container: Option<Container>,
    pub previous: bool,
    pub tail_lines: usize,
    pub since_seconds: Option<u64>,
}

/// `list_events`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Events {
    pub namespace: Option<Namespace>,
    pub kind: Option<KindName>,
    pub name: Option<Name>,
    pub since_seconds: u64,
    pub limit: usize,
}

/// One read, closed over the four tools.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    List(List),
    Get(Get),
    Logs(Logs),
    Events(Events),
}

impl Request {
    /// The tool this request came from.
    pub fn tool(&self) -> &'static str {
        match self {
            Self::List(_) => TOOLS[0],
            Self::Get(_) => TOOLS[1],
            Self::Logs(_) => TOOLS[2],
            Self::Events(_) => TOOLS[3],
        }
    }
}

/// A parsed call: the cluster entry it names (`None` is the session's own)
/// and what it asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub cluster: Option<EntryId>,
    pub request: Request,
}

/// An argument's name as a refusal may repeat it: short and made of the
/// letters a name is made of. Any other name is the agent's text, not a name.
fn named(key: &str) -> String {
    if key.len() <= 32
        && !key.is_empty()
        && key.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
    {
        key.to_owned()
    } else {
        "an argument".to_owned()
    }
}

/// The arguments of a call, once checked for a name given twice.
struct Args(Vec<(String, Value)>);

impl Args {
    fn new(pairs: impl IntoIterator<Item = (String, Value)>) -> Result<Self, Refusal> {
        let mut seen: Vec<(String, Value)> = Vec::new();
        for (key, value) in pairs {
            if seen.iter().any(|(known, _)| *known == key) {
                return Err(Refusal::invalid(&named(&key), "is given twice"));
            }
            seen.push((key, value));
        }
        Ok(Self(seen))
    }

    /// Takes `key` out, so what is left at the end is what the tool does not
    /// know.
    fn take(&mut self, key: &str) -> Option<Value> {
        let at = self.0.iter().position(|(known, _)| known == key)?;
        Some(self.0.remove(at).1)
    }

    fn string(&mut self, key: &str) -> Result<Option<String>, Refusal> {
        match self.take(key) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text)),
            Some(_) => Err(Refusal::invalid(key, "must be a string")),
        }
    }

    fn flag(&mut self, key: &str) -> Result<Option<bool>, Refusal> {
        match self.take(key) {
            None => Ok(None),
            Some(Value::Bool(flag)) => Ok(Some(flag)),
            Some(_) => Err(Refusal::invalid(key, "must be true or false")),
        }
    }

    /// A whole number in `1..=max`.
    fn number(&mut self, key: &str, max: u64) -> Result<Option<u64>, Refusal> {
        match self.take(key) {
            None => Ok(None),
            Some(Value::Number(number)) => match number.as_u64() {
                Some(value) if (1..=max).contains(&value) => Ok(Some(value)),
                Some(_) => Err(Refusal::invalid(key, &format!("must be from 1 to {max}"))),
                None => Err(Refusal::invalid(key, "must be a whole number from 1 up")),
            },
            Some(_) => Err(Refusal::invalid(key, "must be a number")),
        }
    }

    fn parsed<T>(
        &mut self,
        key: &str,
        parse: fn(&str, &str) -> Result<T, Refusal>,
    ) -> Result<Option<T>, Refusal> {
        self.string(key)?
            .map(|value| parse(key, &value))
            .transpose()
    }

    fn required<T>(
        &mut self,
        key: &str,
        parse: fn(&str, &str) -> Result<T, Refusal>,
    ) -> Result<T, Refusal> {
        self.parsed(key, parse)?
            .ok_or_else(|| Refusal::invalid(key, "is required"))
    }

    /// Fails on the first argument left over.
    fn finish(self) -> Result<(), Refusal> {
        match self.0.first() {
            None => Ok(()),
            Some((key, _)) => Err(Refusal::invalid(
                &named(key),
                "is not an argument of this tool",
            )),
        }
    }
}

impl Call {
    /// Parses a tool call whose arguments are a JSON object. A name given
    /// twice cannot survive a JSON object; [`Call::parse_pairs`] checks it
    /// for input that kept the pairs.
    pub fn parse(tool: &str, arguments: &Value) -> Result<Self, Refusal> {
        match arguments {
            Value::Object(map) => {
                Self::parse_pairs(tool, map.iter().map(|(k, v)| (k.clone(), v.clone())))
            }
            Value::Null => Self::parse_pairs(tool, std::iter::empty()),
            _ => Err(Refusal::invalid("arguments", "must be an object")),
        }
    }

    /// Parses a tool call from its arguments as pairs, refusing a name that
    /// appears twice.
    pub fn parse_pairs(
        tool: &str,
        arguments: impl IntoIterator<Item = (String, Value)>,
    ) -> Result<Self, Refusal> {
        let Some(tool) = TOOLS.iter().find(|known| **known == tool) else {
            return Err(Refusal::invalid(
                "tool",
                "is not one of this server's tools",
            ));
        };
        let mut args = Args::new(arguments)?;
        let cluster = args.parsed("cluster", EntryId::parse)?;
        let request = match *tool {
            "list_objects" => Request::List(List {
                kind: args.required("kind", KindName::parse)?,
                namespace: args.parsed("namespace", Namespace::parse)?,
                limit: args
                    .number("limit", MAX_ITEMS as u64)?
                    .map_or(DEFAULT_ITEMS, |n| n as usize),
            }),
            "get_object" => Request::Get(Get {
                kind: args.required("kind", KindName::parse)?,
                namespace: args.parsed("namespace", Namespace::parse)?,
                name: args.required("name", Name::parse)?,
            }),
            "pod_logs" => Request::Logs(Logs {
                namespace: args.required("namespace", Namespace::parse)?,
                pod: args.required("pod", Name::parse)?,
                container: args.parsed("container", Container::parse)?,
                previous: args.flag("previous")?.unwrap_or(false),
                tail_lines: args
                    .number("tail_lines", MAX_LOG_LINES as u64)?
                    .map_or(DEFAULT_LOG_LINES, |n| n as usize),
                since_seconds: args.number("since_seconds", MAX_SINCE_SECONDS)?,
            }),
            _ => Request::Events(Events {
                namespace: args.parsed("namespace", Namespace::parse)?,
                kind: args.parsed("kind", KindName::parse)?,
                name: args.parsed("name", Name::parse)?,
                since_seconds: args
                    .number("since_seconds", MAX_SINCE_SECONDS)?
                    .unwrap_or(DEFAULT_EVENT_SINCE_SECONDS),
                limit: args
                    .number("limit", MAX_ITEMS as u64)?
                    .map_or(DEFAULT_ITEMS, |n| n as usize),
            }),
        };
        args.finish()?;
        Ok(Self { cluster, request })
    }
}
