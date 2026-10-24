//! Which lines of the edited text differ from the text the editor started
//! from, so the editor can mark them. Lines are compared whole, in order.

/// The most cells of the comparison table, which is built on every change of
/// the text; a larger edit marks every line between the text's unchanged
/// start and end.
const LIMIT: usize = 250_000;

/// What the edit did to the lines.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Marks {
    /// The lines of the new text that are not in the old one, by index.
    pub added: Vec<usize>,
    /// How many lines of the old text are gone.
    pub removed: usize,
}

impl Marks {
    pub(super) fn count(&self) -> usize {
        self.added.len().max(self.removed)
    }
}

pub(super) fn compare(original: &[&str], now: &[&str]) -> Marks {
    let start = original
        .iter()
        .zip(now)
        .take_while(|(was, is)| was == is)
        .count();
    let end = original[start..]
        .iter()
        .rev()
        .zip(now[start..].iter().rev())
        .take_while(|(was, is)| was == is)
        .count();
    let (was, is) = (
        &original[start..original.len() - end],
        &now[start..now.len() - end],
    );
    if was.len() * is.len() > LIMIT {
        return Marks {
            added: (start..start + is.len()).collect(),
            removed: was.len(),
        };
    }
    // `table[i][j]`: how many lines `was[i..]` and `is[j..]` share, in order.
    let width = is.len() + 1;
    let mut table = vec![0u32; (was.len() + 1) * width];
    for i in (0..was.len()).rev() {
        for j in (0..is.len()).rev() {
            table[i * width + j] = if was[i] == is[j] {
                table[(i + 1) * width + j + 1] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }
    let mut marks = Marks::default();
    let (mut i, mut j) = (0, 0);
    while i < was.len() && j < is.len() {
        if was[i] == is[j] {
            (i, j) = (i + 1, j + 1);
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            marks.removed += 1;
            i += 1;
        } else {
            marks.added.push(start + j);
            j += 1;
        }
    }
    marks.removed += was.len() - i;
    marks
        .added
        .extend((start + j..start + is.len()).collect::<Vec<_>>());
    marks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<&str> {
        text.lines().collect()
    }

    #[test]
    fn the_changed_and_added_lines_are_marked_and_the_rest_are_not() {
        let was = lines("a\nb\nc\nd\ne\nf");
        let now = lines("a\nB\nc\nd\ne\nf\ng");
        let marks = compare(&was, &now);
        assert_eq!(marks.added, [1, 6]);
        assert_eq!(marks.removed, 1);
        assert_eq!(marks.count(), 2);
    }

    #[test]
    fn two_separate_edits_leave_the_lines_between_them_alone() {
        let was = lines("a\nb\nc\nd\ne");
        let now = lines("A\nb\nc\nd\nE");
        assert_eq!(compare(&was, &now).added, [0, 4]);
    }

    #[test]
    fn a_deleted_line_marks_nothing_but_is_counted_and_no_edit_is_none() {
        let was = lines("a\nb\nc");
        let marks = compare(&was, &lines("a\nc"));
        assert_eq!((marks.added.len(), marks.removed), (0, 1));
        assert_eq!(compare(&was, &was), Marks::default());
    }
}
