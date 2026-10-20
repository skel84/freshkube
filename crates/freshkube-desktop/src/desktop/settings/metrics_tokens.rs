//! Retire a setup token only after its last YAML reference has been saved away.
use freshkube_core::{monitoring::scope, secrets::Secrets, setup::Setup};

pub(super) fn forget_replaced(
    secrets: Option<Secrets>,
    before: Option<Setup>,
    after: Option<Setup>,
    preferences: Option<std::path::PathBuf>,
) -> Result<(), String> {
    let (Some(secrets), Some(before), Some(after), Some(preferences)) =
        (secrets, before, after, preferences)
    else {
        return Ok(());
    };
    let kept = scope::token_urls(&after);
    let mut retired = scope::token_urls(&before);
    retired.retain(|url| !kept.contains(url));
    if retired.is_empty() {
        return Ok(());
    }
    let kept = crate::monitoring::saved_token_urls(&preferences).map_err(|_| {
        "The setup was saved, but monitoring.json couldn’t be checked. Its tokens were kept."
            .to_owned()
    })?;
    for url in retired.difference(&kept) {
        secrets
            .forget(&crate::monitoring::token_account(url))
            .map_err(|_| {
                "The setup was saved, but its replaced token couldn’t be removed from keys.json."
                    .to_owned()
            })?;
    }
    Ok(())
}
