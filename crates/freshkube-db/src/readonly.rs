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
    /// A first word that passes `BEGIN READ ONLY` but does maintenance and
    /// takes exclusive locks: `REINDEX`, `CLUSTER`, `VACUUM`, `ANALYZE`,
    /// `CHECKPOINT`.
    Maintenance(&'static str),
    /// A function that acts on the server or other sessions without writing,
    /// so the read-only transaction lets it through.
    Function(&'static str),
    /// `EXPLAIN ANALYZE` runs the statement it explains.
    Analyze,
    /// `U&"…"`: an identifier written with Unicode escapes can spell a
    /// function's name without the name appearing in the text.
    UnicodeEscape,
}

impl Refusal {
    pub fn explain(self) -> String {
        match self {
            Self::SessionSetting => {
                "This session is read-only. SET and RESET would change how it runs, so they are not sent."
                    .into()
            }
            Self::Transaction => {
                "Each request already runs in its own read-only transaction. BEGIN, COMMIT and ROLLBACK are not sent."
                    .into()
            }
            Self::Several => "One statement at a time.".into(),
            Self::Empty => "Nothing to run.".into(),
            Self::Maintenance(word) => format!(
                "{} {}, which a read-only transaction does not stop, so it is not sent.",
                word.to_uppercase(),
                maintenance_why(word)
            ),
            Self::Function(name) => format!(
                "{name}() {}, which a read-only transaction does not stop, so it is not sent.",
                function_why(name)
            ),
            Self::UnicodeEscape => {
                "A Unicode-escaped identifier (U&\"…\") can spell the name of a function that is not sent, so it is not sent."
                    .into()
            }
            Self::Analyze => {
                "EXPLAIN ANALYZE runs the statement it explains, so it is not sent. Plan shows the plan without running it."
                    .into()
            }
        }
    }
}

fn maintenance_why(word: &str) -> &'static str {
    match word {
        "reindex" => "rebuilds indexes under exclusive locks",
        "cluster" => "rewrites a table under an exclusive lock",
        "vacuum" => "rewrites and locks tables and uses the server's disk and CPU",
        "checkpoint" => "forces a write-heavy checkpoint on the server",
        "do" => "runs a block of code that can build and run other statements",
        "copy" => "can write a file on the server or run a program there",
        _ => "reads whole tables to refresh statistics and takes locks",
    }
}

fn function_why(name: &str) -> &'static str {
    match name {
        "pg_terminate_backend" | "pg_cancel_backend" => "ends other sessions' work",
        "pg_reload_conf" | "pg_rotate_logfile" => "changes how the server runs",
        "pg_notify" => "sends notifications to other sessions",
        "set_config" => "can turn this session's settings, such as read-only, off",
        "lo_import" | "lo_export" => "reads or writes files on the server",
        "dblink" => "opens connections that are not read-only",
        "query_to_xml"
        | "query_to_xmlschema"
        | "query_to_xml_and_xmlschema"
        | "ts_stat"
        | "ts_rewrite" => "runs a statement given as text, which this check cannot read",
        _ => "holds locks that block the application",
    }
}

/// The first words refused: maintenance, and statements that run text built
/// at run time or reach the server's files.
pub(crate) const MAINTENANCE: [&str; 8] = [
    "reindex",
    "cluster",
    "vacuum",
    "analyze",
    "analyse",
    "checkpoint",
    "do",
    "copy",
];

/// Functions refused by name, whole word, anywhere in a statement.
pub(crate) const FUNCTIONS: [&str; 21] = [
    "pg_terminate_backend",
    "pg_cancel_backend",
    "pg_reload_conf",
    "pg_rotate_logfile",
    "pg_notify",
    "set_config",
    "pg_advisory_lock",
    "pg_advisory_lock_shared",
    "pg_advisory_xact_lock",
    "pg_advisory_xact_lock_shared",
    "pg_try_advisory_lock",
    "pg_try_advisory_lock_shared",
    "pg_try_advisory_xact_lock",
    "pg_try_advisory_xact_lock_shared",
    "lo_import",
    "lo_export",
    // These run the text of a statement given as a string, so they can run
    // what the list would refuse by name.
    "query_to_xml",
    "query_to_xmlschema",
    "query_to_xml_and_xmlschema",
    "ts_stat",
    "ts_rewrite",
];

/// Checks one request. `Ok` means only that it is a single statement that does
/// not touch the session; the read-only transaction still decides what it may do.
pub fn check_statement(sql: &str) -> Result<(), Refusal> {
    let statements = split(sql);
    match statements.as_slice() {
        [] => Err(Refusal::Empty),
        [one] => first_word_refusal(one)
            .and_then(|()| explain_analyze(one))
            .and_then(|()| refused_function(one))
            .and_then(|()| unicode_escaped_identifier(one)),
        _ => Err(Refusal::Several),
    }
}

/// The first word of a statement, lowercased, past comments and opening
/// parentheses; empty when there is none.
pub fn leading_word(sql: &str) -> String {
    let statements = split(sql);
    let statement = statements.first().map_or("", String::as_str);
    word_of(statement)
}

fn word_of(statement: &str) -> String {
    statement
        .trim_start_matches(|c: char| c == '(' || c.is_whitespace())
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn first_word_refusal(statement: &str) -> Result<(), Refusal> {
    let word = word_of(statement);
    match word.as_str() {
        "set" | "reset" | "discard" | "load" | "listen" | "unlisten" => {
            Err(Refusal::SessionSetting)
        }
        "begin" | "start" | "commit" | "end" | "rollback" | "abort" | "savepoint" | "release"
        | "prepare" => Err(Refusal::Transaction),
        word => match MAINTENANCE.iter().find(|m| **m == word) {
            Some(found) => Err(Refusal::Maintenance(found)),
            None => Ok(()),
        },
    }
}

/// `EXPLAIN ANALYZE` in any spelling: `explain analyze`, `explain verbose
/// analyse`, `explain (costs off, analyze)`. An option list that mentions
/// ANALYZE at all is refused, even `analyze false`.
fn explain_analyze(statement: &str) -> Result<(), Refusal> {
    if word_of(statement) != "explain" {
        return Ok(());
    }
    let after = statement
        .trim_start_matches(|c: char| c == '(' || c.is_whitespace())
        .get("explain".len()..)
        .unwrap_or("")
        .trim_start();
    let is_analyze = |w: &str| matches!(w.to_ascii_lowercase().as_str(), "analyze" | "analyse");
    let found = if let Some(list) = after.strip_prefix('(') {
        words(list.split(')').next().unwrap_or("")).any(|w| is_analyze(&w))
    } else {
        // Bare options end at the first word that is not one of them; words
        // are split on any whitespace, however many and whichever kind.
        after
            .split_whitespace()
            .take_while(|w| is_analyze(w) || w.eq_ignore_ascii_case("verbose"))
            .any(is_analyze)
    };
    if found { Err(Refusal::Analyze) } else { Ok(()) }
}

/// `U&"…"` outside string literals. Text in single quotes, E-strings and
/// dollar quotes is skipped (a `U&"` there is only text); a double-quoted
/// identifier is skipped as a whole. A `u` that ends a longer word (`fooU&"x"`
/// is `fooU & "x"`) is not the prefix.
fn unicode_escaped_identifier(statement: &str) -> Result<(), Refusal> {
    let chars: Vec<char> = statement.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' | '"' => i = quoted_end(&chars, i),
            '$' if !prev_is_ident(&chars, i) => match skip_dollar(&chars, i) {
                Some(end) => i = end,
                None => i += 1,
            },
            'u' | 'U'
                if chars.get(i + 1) == Some(&'&')
                    && chars.get(i + 2) == Some(&'"')
                    && !prev_is_ident(&chars, i) =>
            {
                return Err(Refusal::UnicodeEscape);
            }
            _ => i += 1,
        }
    }
    Ok(())
}

/// End (exclusive) of the quoted run starting at `start`, which is a `'` or a
/// `"`. After a standalone `e` or `E` a single quote opens an E-string, where
/// a backslash escapes the next character (`E'\''` is one quote); otherwise
/// only a doubled quote does. Every scan of a statement's text uses this one
/// rule, so they agree on where a string ends.
pub(crate) fn quoted_end(chars: &[char], start: usize) -> usize {
    let quote = chars[start];
    let e_string = quote == '\''
        && start > 0
        && matches!(chars[start - 1], 'e' | 'E')
        && !(start > 1 && prev_is_ident(chars, start - 1));
    if e_string {
        skip_estring(chars, start)
    } else {
        skip_quoted(chars, start, quote)
    }
}

/// End of an E'…' string: a backslash escapes the next character.
fn skip_estring(chars: &[char], start: usize) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '\'' if chars.get(i + 1) == Some(&'\'') => i += 2,
            '\'' => return i + 1,
            _ => i += 1,
        }
    }
    chars.len()
}

/// Lowercased identifier words of `text`: runs of letters, digits, `_` and `$`.
fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
}

/// A refused function named anywhere in the statement, as a whole word. The
/// text is scanned with its comments gone and its strings in: a function
/// run from a `DO` block's dollar quote is named inside one.
fn refused_function(statement: &str) -> Result<(), Refusal> {
    for word in words(statement) {
        if let Some(found) = FUNCTIONS.iter().find(|f| **f == word) {
            return Err(Refusal::Function(found));
        }
        if word.starts_with("dblink") {
            return Err(Refusal::Function("dblink"));
        }
    }
    Ok(())
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
                let end = quoted_end(&chars, i);
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

pub(crate) fn prev_is_ident(chars: &[char], i: usize) -> bool {
    i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_' || chars[i - 1] == '$')
}

fn push(out: &mut Vec<String>, current: &mut String) {
    let piece = std::mem::take(current);
    if !piece.trim().is_empty() {
        out.push(piece);
    }
}

/// End (exclusive) of the quoted run starting at `start`; doubled quotes stay inside.
pub(crate) fn skip_quoted(chars: &[char], start: usize, quote: char) -> usize {
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
pub(crate) fn skip_dollar(chars: &[char], start: usize) -> Option<usize> {
    // A tag is an identifier: `$1$` is not one (`$1` is a parameter).
    if chars.get(start + 1).is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
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

    #[test]
    fn maintenance_first_words_are_refused_with_their_reason() {
        for (sql, word) in [
            ("REINDEX TABLE orders", "reindex"),
            ("cluster orders using orders_pkey", "cluster"),
            ("VACUUM FULL orders", "vacuum"),
            ("analyze orders", "analyze"),
            ("ANALYSE", "analyse"),
            ("  /* x */ checkpoint", "checkpoint"),
        ] {
            let refusal = check_statement(sql).unwrap_err();
            assert_eq!(refusal, Refusal::Maintenance(word), "{sql}");
            assert!(
                refusal.explain().starts_with(&word.to_uppercase()),
                "{}",
                refusal.explain()
            );
        }
        // A column or table named like one is not a first word.
        assert_eq!(check_statement("select vacuum from t"), Ok(()));
    }

    #[test]
    fn refused_functions_are_refused_as_whole_words_in_any_case_anywhere() {
        for name in FUNCTIONS {
            for sql in [
                format!("select {name}(1)"),
                format!("SELECT {}(1)", name.to_uppercase()),
                format!("select 1 where exists (select {name}(2))"),
                format!("select pg_catalog.{name}(1)"),
                format!("select \"{name}\"(1)"),
                format!("select (select {name}(1))"),
            ] {
                assert_eq!(check_statement(&sql), Err(Refusal::Function(name)), "{sql}");
            }
        }
        for sql in [
            "select dblink('host=x', 'select 1')",
            "select dblink_connect('x')",
            "select * from DBLINK_EXEC('x','y')",
        ] {
            assert_eq!(
                check_statement(sql),
                Err(Refusal::Function("dblink")),
                "{sql}"
            );
        }
        let words = Refusal::Function("set_config").explain();
        assert!(
            words.contains("set_config()") && words.contains("read-only"),
            "{words}"
        );
    }

    #[test]
    fn a_longer_name_is_not_a_refused_function_and_a_comment_is_not_a_call() {
        for sql in [
            "select my_set_config(1)",
            "select pg_notify_all",
            "select set_config_value from t",
            "select 1 -- pg_notify('x', 'y')",
            "select /* pg_terminate_backend(1) */ 1",
        ] {
            assert_eq!(check_statement(sql), Ok(()), "{sql}");
        }
    }

    #[test]
    fn explain_analyze_is_refused_in_every_spelling_and_a_plain_explain_is_not() {
        for sql in [
            "explain analyze select 1",
            "EXPLAIN ANALYSE select 1",
            "explain (analyze) select 1",
            "explain (analyse true, costs off) select 1",
            "explain (costs off, ANALYZE) select 1",
            "explain verbose analyze select 1",
            "explain analyze verbose select 1",
            "explain (analyze false) select 1",
        ] {
            assert_eq!(check_statement(sql), Err(Refusal::Analyze), "{sql}");
        }
        for sql in [
            "explain select 1",
            "explain verbose select 1",
            "explain (costs off, format text) select 1",
            "explain select 'analyze'",
            "explain select analyze from t",
        ] {
            assert_eq!(check_statement(sql), Ok(()), "{sql}");
        }
    }

    #[test]
    fn statements_that_run_text_built_at_run_time_or_reach_files_are_refused_by_first_word() {
        for (sql, word) in [
            (
                "do $$ begin execute 'select pg_cancel' || '_backend(1)'; end $$",
                "do",
            ),
            ("DO LANGUAGE plpgsql $$ begin null; end $$", "do"),
            ("copy orders to '/tmp/orders.csv'", "copy"),
            ("COPY (select 1) TO PROGRAM 'id'", "copy"),
        ] {
            assert_eq!(
                check_statement(sql),
                Err(Refusal::Maintenance(word)),
                "{sql}"
            );
        }
    }

    #[test]
    fn functions_that_run_a_statement_given_as_text_are_refused() {
        for sql in [
            "select query_to_xml('select pg_term'||'inate_backend(1)', true, false, '')",
            "select QUERY_TO_XMLSCHEMA('select 1', true, false, '')",
            "select query_to_xml_and_xmlschema('select 1', true, false, '')",
            "select * from ts_stat('select to_tsvector(''x'')')",
        ] {
            assert!(
                matches!(check_statement(sql), Err(Refusal::Function(_))),
                "{sql}"
            );
        }
    }

    #[test]
    fn a_unicode_escaped_identifier_is_refused_outside_strings_only() {
        for sql in [
            "select U&\"pg_terminate_backen\\0064\"(pid) from pg_stat_activity",
            "select u&\"x\" from t",
            "select 1 where (u&\"y\") is null",
            // After a string that only looks like it ends the text.
            "select E'\\'', u&\"x\"",
        ] {
            assert_eq!(check_statement(sql), Err(Refusal::UnicodeEscape), "{sql}");
        }
        for sql in [
            "select 'U&\"x\"'",
            "select $$u&\"x\"$$",
            "select fooU&\"x\" from t",
            "select \"U&\" from t",
            "select U&'d\\0061t\\+000061'",
        ] {
            assert_eq!(check_statement(sql), Ok(()), "{sql}");
        }
    }

    #[test]
    fn explain_options_are_read_across_any_whitespace_without_panicking() {
        for sql in [
            "explain verbose   analyze select 1",
            "explain   analyze   select 1",
            "explain\tverbose\n  analyze select 1",
            "explain\u{a0}analyze select 1",
            "explain verbose\u{a0}\u{a0}analyse select 1",
            "explain\u{a0}(analyze) select 1",
        ] {
            assert_eq!(check_statement(sql), Err(Refusal::Analyze), "{sql:?}");
        }
        for sql in [
            "explain\u{a0}select 1",
            "explain verbose\u{a0}select analyze from t",
            "explain\u{2003}\u{2003}select 1",
        ] {
            assert_eq!(check_statement(sql), Ok(()), "{sql:?}");
        }
    }

    #[test]
    fn an_e_string_with_an_escaped_quote_does_not_hide_the_rest_of_the_statement() {
        // The server reads `E'\''` as one quote; reading it as an empty string
        // plus a quote would hide the call between the two `--`-like pieces.
        let sql = "select E'\\'', 'x--', pg_terminate_backend(1), U&\"x\"\nfrom pg_stat_activity";
        assert_eq!(
            check_statement(sql),
            Err(Refusal::Function("pg_terminate_backend"))
        );
        // The same text with a standard string keeps its meaning: a backslash
        // is plain there, and the quote ends the string.
        assert_eq!(check_statement("select '\\', 1"), Ok(()));
        // Only a standalone `e` makes an E-string.
        assert_eq!(check_statement("select name'\\' from t"), Ok(()));
        // A semicolon inside an E-string does not end the statement.
        assert_eq!(check_statement("select E'a\\';b'"), Ok(()));
        // And a `--` inside one is not a comment.
        assert_eq!(
            check_statement("select E'\\'--', pg_notify('a','b')"),
            Err(Refusal::Function("pg_notify"))
        );
    }

    #[test]
    fn ts_rewrite_runs_a_statement_given_as_text_and_is_refused() {
        assert_eq!(
            check_statement(
                "select ts_rewrite('a'::tsquery, 'select pg_term'||'inate_backend(1)')"
            ),
            Err(Refusal::Function("ts_rewrite"))
        );
    }

    #[test]
    fn a_dollar_tag_cannot_start_with_a_digit() {
        // `$1$` is the parameter `$1` and a `$`: not a quote, so what follows
        // is text the server reads, and the call after it is a call.
        assert_eq!(
            check_statement("select $1$, 'a', pg_terminate_backend(1), $1$"),
            Err(Refusal::Function("pg_terminate_backend"))
        );
        assert_eq!(
            check_statement("select $1$ u&\"x\" $1$"),
            Err(Refusal::UnicodeEscape)
        );
        // A tag that starts with a letter or an underscore still quotes: the
        // `U&"` scan skips what is inside.
        assert_eq!(check_statement("select $a1$ u&\"x\" $a1$"), Ok(()));
        assert_eq!(check_statement("select $_$ u&\"x\" $_$"), Ok(()));
        let chars: Vec<char> = "$1$ x $1$".chars().collect();
        assert_eq!(skip_dollar(&chars, 0), None);
    }
}
