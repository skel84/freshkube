//! The pure function from what was read to the applications.

use super::overrides::apply;
use super::rules::{Builder, argo, coverage, infer_across_sessions, kargo, part_of};
use super::{Derived, Inputs, Override};

/// Applications from what was read, in this order:
///
/// 1. a Kargo Project;
/// 2. an Argo CD ApplicationSet or Application;
/// 3. the `part-of` label;
/// 4. the user's override, which beats every rule above: a rename, merge,
///    split or hide applies after all of them, to a Kargo Project's
///    application as to any other.
///
/// The same inputs always give the same answer, in the same order.
pub fn derive(inputs: &Inputs, over: &Override) -> Derived {
    let mut builder = Builder::default();
    let projects = kargo(&mut builder, inputs);
    let home = argo(&mut builder, inputs, &projects);
    part_of(&mut builder, inputs, &home);
    infer_across_sessions(&mut builder);
    let applied = apply(builder.apps, over);
    let by_name = |apps: &mut Vec<super::Application>| {
        apps.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
    };
    let (mut applications, mut hidden) = (applied.shown, applied.hidden);
    by_name(&mut applications);
    by_name(&mut hidden);
    Derived {
        applications,
        hidden,
        coverage: coverage(inputs),
        unmatched: applied.unmatched,
    }
}
