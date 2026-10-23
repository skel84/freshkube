//! Statements a read-only session refuses before they are sent.
//!
//! The session is what enforces read-only (`default_transaction_read_only` at
//! connect and `BEGIN READ ONLY` per request). This check only keeps a request
//! from changing the session's own state, and explains the refusal in words.
//! `SET`, `BEGIN` and `COMMIT` would let a statement leave the read-only
//! transaction or turn the setting off, so they and their relatives never go.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// `SET`, `RESET`, `SET ROLE`, `SET TRANSACTION`, `SET SESSION AUTHORIZATION`.
    SessionSetting,
    /// `BEGIN`, `START TRANSACTION`, `COMMIT`, `END`, `ROLLBACK`, `ABORT`,
    /// `SAVEPOINT`, `RELEASE`, `PREPARE TRANSACTION`.
    Transaction,
    /// More than one statement in one request.
    Several,
    Empty,
}

impl Refusal {
    pub fn explain(self) -> &'static str {
        match self {
            Self::SessionSetting => {
                "This session is read-only. SET and RESET would change how it runs, so they are not sent."
            }
            Self::Transaction => {
                "Each request already runs in its own read-only transaction. BEGIN, COMMIT and ROLLBACK are not sent."
            }
            Self::Several => "One statement at a time.",
            Self::Empty => "Nothing to run.",
        }
    }
}

/// Checks one request. `Ok` means only that it is a single statement that does
/// not touch the session; the read-only transaction still decides what it may do.
pub fn check_statement(sql: &str) -> Result<(), Refusal> {
    let statements = split(sql);
    match statements.as_slice() {
        [] => Err(Refusal::Empty),
        [one] => first_word_refusal(one),
        _ => Err(Refusal::Several),
    }
}

fn first_word_refusal(statement: &str) -> Result<(), Refusal> {
    let word: String = statement
        .trim_start_matches(|c: char| c == '(' || c.is_whitespace())
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_lowercase();
    match word.as_str() {
        "set" | "reset" | "discard" | "load" | "listen" | "unlisten" => {
            Err(Refusal::SessionSetting)
        }
        "begin" | "start" | "commit" | "end" | "rollback" | "abort" | "savepoint" | "release"
        | "prepare" => Err(Refusal::Transaction),
        _ => Ok(()),
    }
}

/// Splits on `;` outside quotes, identifiers, dollar quotes and comments, and
/// drops empty pieces and comment-only pieces.
fn split(sql: &str) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' | '"' => {
                let end = skip_quoted(&chars, i, c);
                current.extend(&chars[i..end]);
                i = end;
                continue;
            }
            '-' if chars.get(i + 1) == Some(&'-') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                let mut depth = 0;
                while i < chars.len() {
                    if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                        depth += 1;
                        i += 2;
                    } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                current.push(' ');
                continue;
            }
            // A `$` after an identifier character belongs to the identifier
            // (`a$b$`), not to a dollar-quoted string.
            '$' if !prev_is_ident(&chars, i) => {
                if let Some(end) = skip_dollar(&chars, i) {
                    current.extend(&chars[i..end]);
                    i = end;
                    continue;
                }
            }
            ';' => {
                push(&mut out, &mut current);
                i += 1;
                continue;
            }
            _ => {}
        }
        current.push(c);
        i += 1;
    }
    push(&mut out, &mut current);
    out
}

fn prev_is_ident(chars: &[char], i: usize) -> bool {
    i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_' || chars[i - 1] == '$')
}

fn push(out: &mut Vec<String>, current: &mut String) {
    let piece = std::mem::take(current);
    if !piece.trim().is_empty() {
        out.push(piece);
    }
}

/// End (exclusive) of the quoted run starting at `start`; doubled quotes stay inside.
fn skip_quoted(chars: &[char], start: usize, quote: char) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        if chars[i] == quote {
            if chars.get(i + 1) == Some(&quote) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    chars.len()
}

/// End of a `$tag$ ... $tag$` string starting at `start`, if one starts there.
fn skip_dollar(chars: &[char], start: usize) -> Option<usize> {
    let mut j = start + 1;
    while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
        j += 1;
    }
    if chars.get(j) != Some(&'$') {
        return None;
    }
    let tag: Vec<char> = chars[start..=j].to_vec();
    let mut k = j + 1;
    while k + tag.len() <= chars.len() {
        if chars[k..k + tag.len()] == tag[..] {
            return Some(k + tag.len());
        }
        k += 1;
    }
    Some(chars.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_pass() {
        for sql in [
            "SELECT 1",
            "select * from t;",
            "  (select 1) union (select 2)",
            "WITH x AS (SELECT 1) SELECT * FROM x",
            "EXPLAIN SELECT 1",
            "SHOW server_version",
            "SELECT ';' , \"a;b\", $$x;y$$, $t$;$t$",
            "SELECT 1 -- ; SET x\n",
            "SELECT /* ; commit */ 1",
        ] {
            assert_eq!(check_statement(sql), Ok(()), "{sql}");
        }
    }

    #[test]
    fn set_begin_and_commit_are_refused_in_any_spelling() {
        assert_eq!(
            check_statement("SET default_transaction_read_only = off"),
            Err(Refusal::SessionSetting)
        );
        assert_eq!(
            check_statement("  set role postgres"),
            Err(Refusal::SessionSetting)
        );
        assert_eq!(check_statement("RESET ALL"), Err(Refusal::SessionSetting));
        assert_eq!(check_statement("/* x */ BEGIN"), Err(Refusal::Transaction));
        assert_eq!(
            check_statement("start transaction read write"),
            Err(Refusal::Transaction)
        );
        assert_eq!(check_statement("COMMIT;"), Err(Refusal::Transaction));
        assert_eq!(check_statement("end"), Err(Refusal::Transaction));
        assert_eq!(check_statement("ROLLBACK"), Err(Refusal::Transaction));
        assert_eq!(check_statement("SAVEPOINT a"), Err(Refusal::Transaction));
    }

    #[test]
    fn a_dollar_inside_an_identifier_does_not_open_a_quote() {
        let sql =
            "SELECT 1 AS a$b$; COMMIT; SET TRANSACTION READ WRITE; DELETE FROM t; SELECT 1 AS c$b$";
        assert_eq!(check_statement(sql), Err(Refusal::Several));
        // A real dollar quote still hides its semicolons.
        assert_eq!(check_statement("SELECT $b$; COMMIT;$b$"), Ok(()));
    }

    #[test]
    fn a_second_statement_is_refused_even_when_the_first_is_harmless() {
        assert_eq!(check_statement("SELECT 1; COMMIT"), Err(Refusal::Several));
        assert_eq!(check_statement("SELECT 1; SELECT 2"), Err(Refusal::Several));
        assert_eq!(check_statement(""), Err(Refusal::Empty));
        assert_eq!(check_statement(" ; -- nothing\n"), Err(Refusal::Empty));
    }
}
