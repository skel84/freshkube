//! The label matcher that scopes a shared store's reads to one entry.
//!
//! A central store holds every cluster's series, told apart by a label such
//! as `cluster`. Without a matcher on every selector, a dashboard for one
//! entry draws another's numbers. [`Matcher::scope_request`] rewrites a
//! planned request before it is sent: every vector and matrix selector in
//! its PromQL gets `label="value"`, replacing any matcher the query already
//! had on that label. A query it can't parse or can't scope is never sent.
use grafaui_prometheus::api::ApiRequest;
use promql_parser::{
    label::{MatchOp, Matcher as Selector},
    parser::{self, Expr, VectorSelector},
};

/// One label and the value that selects an entry's series.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Matcher {
    label: String,
    value: String,
}

impl Matcher {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    /// `{label="value"}`, the selector that matches every series of the
    /// entry.
    pub fn selector(&self) -> String {
        format!("{{{}}}", self.pair())
    }

    fn pair(&self) -> String {
        let value = self.value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("{}=\"{value}\"", self.label)
    }

    /// The query with this matcher on every selector.
    // String exception: the parser's message is shown as the reason a query isn't sent.
    pub fn scope_query(&self, query: &str) -> Result<String, String> {
        let mut expr = parser::parse(query).map_err(|error| format!("{error}"))?;
        self.scope_expr(&mut expr)?;
        Ok(expr.to_string())
    }

    /// The request with its PromQL scoped. Requests that carry none
    /// (`status/buildinfo`, `targets`) pass unchanged; a path this doesn't
    /// know is refused rather than sent unscoped.
    // String exception: the reason is wrapped into a QueryError by the caller.
    pub(super) fn scope_request(&self, request: &ApiRequest) -> Result<ApiRequest, String> {
        let mut scoped = request.clone();
        match request.path.as_str() {
            "query" | "query_range" | "query_exemplars" => {
                for (name, value) in &mut scoped.params {
                    if name == "query" {
                        *value = self.scope_query(value)?;
                    }
                }
            }
            "series" | "labels" => self.scope_matches(&mut scoped)?,
            path if path.starts_with("label/") && path.ends_with("/values") => {
                self.scope_matches(&mut scoped)?
            }
            "status/buildinfo" | "targets" => {}
            path => return Err(format!("it doesn't know how to scope {path}")),
        }
        Ok(scoped)
    }

    /// Every `match[]` scoped, or one added that selects the entry's series.
    fn scope_matches(&self, request: &mut ApiRequest) -> Result<(), String> {
        let mut found = false;
        for (name, value) in &mut request.params {
            if name == "match[]" {
                *value = self.scope_query(value)?;
                found = true;
            }
        }
        if !found {
            request.params.push(("match[]".into(), self.selector()));
        }
        Ok(())
    }

    fn scope_expr(&self, expr: &mut Expr) -> Result<(), String> {
        match expr {
            Expr::VectorSelector(selector) => self.scope_selector(selector),
            Expr::MatrixSelector(matrix) => self.scope_selector(&mut matrix.vs),
            Expr::Aggregate(aggregate) => {
                self.scope_expr(&mut aggregate.expr)?;
                match &mut aggregate.param {
                    Some(param) => self.scope_expr(param),
                    None => Ok(()),
                }
            }
            Expr::Unary(unary) => self.scope_expr(&mut unary.expr),
            Expr::Binary(binary) => {
                self.scope_expr(&mut binary.lhs)?;
                self.scope_expr(&mut binary.rhs)
            }
            Expr::Paren(paren) => self.scope_expr(&mut paren.expr),
            Expr::Subquery(subquery) => self.scope_expr(&mut subquery.expr),
            Expr::Call(call) => call
                .args
                .args
                .iter_mut()
                .try_for_each(|argument| self.scope_expr(argument)),
            Expr::NumberLiteral(_) | Expr::StringLiteral(_) => Ok(()),
            Expr::Extension(_) => Err("the query has an extension it can't scope".into()),
        }
    }

    fn scope_selector(&self, selector: &mut VectorSelector) -> Result<(), String> {
        if !selector.matchers.or_matchers.is_empty() {
            return Err("a selector with `or` can't be scoped".into());
        }
        selector
            .matchers
            .matchers
            .retain(|matcher| matcher.name != self.label);
        selector
            .matchers
            .matchers
            .push(Selector::new(MatchOp::Equal, &self.label, &self.value));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cluster(value: &str) -> Matcher {
        Matcher::new("cluster", value)
    }

    #[test]
    fn every_selector_gets_the_matcher() {
        let scoped = cluster("cluster-a")
            .scope_query(r#"sum by (pod) (rate(container_cpu_usage_seconds_total{namespace="shop"}[5m])) / on (pod) kube_pod_info"#)
            .unwrap();
        assert_eq!(
            scoped.matches(r#"cluster="cluster-a""#).count(),
            2,
            "{scoped}"
        );
        assert!(scoped.contains(r#"namespace="shop""#));
    }

    #[test]
    fn a_matcher_on_the_same_label_is_replaced_whatever_its_operator() {
        for query in [
            r#"up{cluster="cluster-b"}"#,
            r#"up{cluster=~".+"}"#,
            r#"up{cluster!="cluster-a"}"#,
            r#"up{cluster!~"cluster-a"}"#,
        ] {
            let scoped = cluster("cluster-a").scope_query(query).unwrap();
            assert_eq!(scoped, r#"up{cluster="cluster-a"}"#, "{query}");
        }
    }

    #[test]
    fn subqueries_offsets_and_at_keep_their_modifiers() {
        let scoped = cluster("cluster-a")
            .scope_query("max_over_time(rate(up[1m])[10m:1m] offset 5m)")
            .unwrap();
        assert!(
            scoped.contains(r#"up{cluster="cluster-a"}[1m]"#),
            "{scoped}"
        );
        assert!(scoped.contains("offset 5m"), "{scoped}");
        let at = cluster("cluster-a").scope_query("up @ 1000").unwrap();
        assert_eq!(at, r#"up{cluster="cluster-a"} @ 1000.000"#);
    }

    #[test]
    fn a_query_without_a_selector_is_unchanged() {
        assert_eq!(cluster("cluster-a").scope_query("1").unwrap(), "1");
    }

    #[test]
    fn an_unparsable_query_or_an_or_selector_is_refused() {
        assert!(cluster("cluster-a").scope_query("sum(").is_err());
        assert!(
            cluster("cluster-a")
                .scope_query(r#"up{job="a" or job="b"}"#)
                .is_err()
        );
    }

    #[test]
    fn a_value_with_quotes_is_escaped() {
        let scoped = cluster(r#"a"b"#).scope_query("up").unwrap();
        assert_eq!(scoped, r#"up{cluster="a\"b"}"#);
    }

    fn request(path: &str, params: &[(&str, &str)]) -> ApiRequest {
        ApiRequest::new(
            path,
            params
                .iter()
                .map(|(name, value)| ((*name).into(), (*value).into()))
                .collect(),
        )
    }

    #[test]
    fn query_paths_scope_their_query_and_keep_the_rest() {
        for path in ["query", "query_range", "query_exemplars"] {
            let scoped = cluster("cluster-a")
                .scope_request(&request(path, &[("query", "up"), ("time", "10")]))
                .unwrap();
            assert_eq!(
                scoped.params,
                vec![
                    ("query".to_owned(), r#"up{cluster="cluster-a"}"#.to_owned()),
                    ("time".to_owned(), "10".to_owned())
                ]
            );
        }
    }

    #[test]
    fn label_and_series_paths_scope_every_match_or_add_one() {
        let matcher = cluster("cluster-a");
        for path in ["series", "labels", "label/pod/values"] {
            let scoped = matcher
                .scope_request(&request(path, &[("match[]", "up"), ("match[]", "node_up")]))
                .unwrap();
            let matches: Vec<_> = scoped
                .params
                .iter()
                .filter(|(name, _)| name == "match[]")
                .map(|(_, value)| value.as_str())
                .collect();
            assert_eq!(
                matches,
                [
                    r#"up{cluster="cluster-a"}"#,
                    r#"node_up{cluster="cluster-a"}"#
                ]
            );
            let bare = matcher
                .scope_request(&request(path, &[("start", "1")]))
                .unwrap();
            assert!(
                bare.params
                    .contains(&("match[]".into(), r#"{cluster="cluster-a"}"#.into())),
                "{path}"
            );
        }
    }

    #[test]
    fn the_probe_and_the_build_info_stay_unscoped_and_an_unknown_path_is_refused() {
        let matcher = cluster("cluster-a");
        for scoped in [
            matcher
                .scope_request(&request("query", &[("query", "1")]))
                .unwrap(),
            matcher
                .scope_request(&request("status/buildinfo", &[]))
                .unwrap(),
            matcher
                .scope_request(&request("targets", &[("state", "active")]))
                .unwrap(),
        ] {
            assert!(
                !scoped
                    .params
                    .iter()
                    .any(|(_, value)| value.contains("cluster")),
                "{scoped:?}"
            );
        }
        assert!(matcher.scope_request(&request("rules", &[])).is_err());
    }
}
