//! The checks a typed base URL must pass: `http` or `https`, a host, no user,
//! password, query or fragment.

use url::Url;

/// Why a base URL was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BaseUrlError {
    NotAUrl,
    Scheme,
    NoHost,
    UserInfo,
    QueryOrFragment,
}

/// Parses `text` as given (callers trim) and checks it.
pub(crate) fn parse_base_url(text: &str) -> Result<Url, BaseUrlError> {
    let url = Url::parse(text).map_err(|_| BaseUrlError::NotAUrl)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(BaseUrlError::Scheme);
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(BaseUrlError::NoHost);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(BaseUrlError::UserInfo);
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(BaseUrlError::QueryOrFragment);
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_refusal_names_its_reason() {
        for (text, error) in [
            ("not a url", BaseUrlError::NotAUrl),
            ("prom.lan:9090", BaseUrlError::Scheme),
            ("ftp://prom.example.test", BaseUrlError::Scheme),
            (
                "https://user:secret@prom.example.test",
                BaseUrlError::UserInfo,
            ),
            (
                "https://prom.example.test/?token=x",
                BaseUrlError::QueryOrFragment,
            ),
            (
                "https://prom.example.test/#top",
                BaseUrlError::QueryOrFragment,
            ),
        ] {
            assert_eq!(parse_base_url(text).unwrap_err(), error, "{text}");
        }
        assert!(parse_base_url("https://prom.example.test:8481/x").is_ok());
    }
}
