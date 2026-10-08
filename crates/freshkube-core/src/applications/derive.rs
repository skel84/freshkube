//! The pure function from what was read to the applications.

use super::overrides::apply;
use super::rules::{Builder, argo, coverage, kargo, part_of};
use super::{Derived, Inputs, Override};

/// Applications from what was read, by precedence (Kargo Project, Argo CD,
/// `part-of`) and then the override. The same inputs always give the same
/// answer, in the same order.
pub fn derive(inputs: &Inputs, over: &Override) -> Derived {
    let mut builder = Builder::default();
    let projects = kargo(&mut builder, inputs);
    let home = argo(&mut builder, inputs, &projects);
    part_of(&mut builder, inputs, &home);
    for app in builder.apps.values_mut() {
        app.members.sort_by(|a, b| a.at.order().cmp(&b.at.order()));
    }
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
