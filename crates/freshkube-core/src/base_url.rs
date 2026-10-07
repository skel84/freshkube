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
