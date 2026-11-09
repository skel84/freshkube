//! Where the statements of an editor's text are, for "run the selection, else
//! the statement at the cursor" (DBFlux's run semantics).
//!
//! Offsets are byte offsets into the text, so an editor can select the range
//! it returns. Statements end at a `;` outside quotes, identifiers, dollar
//! quotes and comments; a piece with nothing but whitespace and comments is
//! not a statement. This only finds statements. Whether one may run is the
//! session's to decide, and the server's one-statement check is what
//! enforces it ([`crate::postgres`]).
use crate::readonly::{prev_is_ident, quoted_end, skip_dollar};
use std::ops::Range;

/// One statement: its text range, without the `;` that ends it, and the
/// range of the text it owns for placing a cursor (from the end of the
/// statement before, through its own `;`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statement {
    pub range: Range<usize>,
    reach: Range<usize>,
}

/// The statements of `text`, in order. A comment before a statement stays
/// with it; the range is trimmed of surrounding whitespace.
pub fn statements(text: &str) -> Vec<Statement> {
    let chars: Vec<char> = text.chars().collect();
    let mut at: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
    at.push(text.len());
    let mut out = Vec::new();
    let mut piece_start = 0usize;
    let mut content = false;
    let mut i = 0;
    let close = |from: usize, to: usize, after: usize, content: bool, out: &mut Vec<_>| {
        if content {
            let piece = &text[at[from]..at[to]];
            let lead = piece.len() - piece.trim_start().len();
            let trimmed = piece.trim();
            let start = at[from] + lead;
            out.push(Statement {
                range: start..start + trimmed.len(),
                reach: at[from]..at[after],
            });
        }
    };
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' | '"' => {
                content = true;
                i = quoted_end(&chars, i);
            }
            '-' if chars.get(i + 1) == Some(&'-') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
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
            }
            '$' if !prev_is_ident(&chars, i) => {
                content = true;
                i = skip_dollar(&chars, i).unwrap_or(i + 1);
            }
            ';' => {
                close(piece_start, i, i + 1, content, &mut out);
                piece_start = i + 1;
                content = false;
                i += 1;
            }
            c => {
                content |= !c.is_whitespace();
                i += 1;
            }
        }
    }
    close(piece_start, chars.len(), chars.len(), content, &mut out);
    out
}

/// What a run of `text` covers: the selection, if it holds anything, else
/// the statement the cursor is in. `selection` and `cursor` are byte
/// offsets; a selection that is only whitespace counts as none.
pub fn to_run(text: &str, selection: Range<usize>, cursor: usize) -> Vec<Range<usize>> {
    let all = statements(text);
    if selection.start < selection.end && selection.end <= text.len() {
        let picked = &text[selection.clone()];
        if !picked.trim().is_empty() {
            // The selection's own statements, cut as the whole text would be
            // so a selection that starts mid-statement still runs what it holds.
            return statements(picked)
                .into_iter()
                .map(|s| selection.start + s.range.start..selection.start + s.range.end)
                .collect();
        }
    }
    let at = cursor.min(text.len());
    all.iter()
        .find(|s| at <= s.reach.end)
        .or_else(|| all.last())
        .map(|s| vec![s.range.clone()])
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(sql: &str) -> Vec<&str> {
        statements(sql).into_iter().map(|s| &sql[s.range]).collect()
    }

    #[test]
    fn statements_split_outside_quotes_and_comments() {
        assert_eq!(
            texts("SELECT ';', \"a;b\", $$x;y$$; select 2 -- ; no\n; /* only a comment */ ;"),
            vec!["SELECT ';', \"a;b\", $$x;y$$", "select 2 -- ; no"]
        );
        assert!(statements(" ; -- nothing\n ").is_empty());
        assert_eq!(texts("  select 1  "), vec!["select 1"]);
    }

    #[test]
    fn a_dollar_in_an_identifier_does_not_hide_the_next_semicolon() {
        assert_eq!(
            texts("SELECT 1 AS a$b$; SELECT 2 AS c$b$"),
            vec!["SELECT 1 AS a$b$", "SELECT 2 AS c$b$"]
        );
    }

    #[test]
    fn offsets_are_bytes_so_non_ascii_text_selects_cleanly() {
        let sql = "select 'é'; select 'ü'";
        assert_eq!(texts(sql), vec!["select 'é'", "select 'ü'"]);
    }

    #[test]
    fn with_no_selection_the_statement_at_the_cursor_runs() {
        let sql = "select 1;\nselect 2;\nselect 3";
        let run = |cursor| {
            to_run(sql, 0..0, cursor)
                .into_iter()
                .map(|r| &sql[r])
                .collect::<Vec<_>>()
        };
        assert_eq!(run(0), vec!["select 1"]);
        // Right after a statement's semicolon is still that statement.
        assert_eq!(run(9), vec!["select 1"]);
        assert_eq!(run(14), vec!["select 2"]);
        assert_eq!(run(sql.len()), vec!["select 3"]);
        // Past the last statement's end, the last one.
        assert_eq!(to_run("select 1;\n\n", 0..0, 11).len(), 1);
        assert!(to_run("  ", 0..0, 1).is_empty());
    }

    #[test]
    fn a_selection_runs_its_statements_one_by_one() {
        let sql = "select 1; select 2; select 3";
        let picked = to_run(sql, 0..19, 0);
        assert_eq!(
            picked.iter().map(|r| &sql[r.clone()]).collect::<Vec<_>>(),
            vec!["select 1", "select 2"]
        );
        // A selection of part of a statement runs that part.
        let part = to_run(sql, 7..9, 0);
        assert_eq!(&sql[part[0].clone()], "1");
        // A blank selection counts as none.
        assert_eq!(to_run(sql, 9..10, 0).len(), 1);
    }

    #[test]
    fn an_e_string_with_an_escaped_quote_keeps_its_semicolon_inside() {
        // `E'\''` is one quote, so the `;` after it is still inside the string.
        let sql = "select E'\\';x'; select 2";
        assert_eq!(texts(sql), vec!["select E'\\';x'", "select 2"]);
        // A standard string ends at its quote, whatever precedes it.
        assert_eq!(
            texts("select '\\'; select 2"),
            vec!["select '\\'", "select 2"]
        );
    }
}
