//! Retire a setup token only after its last YAML reference has been saved away.
use freshkube_core::{monitoring::scope, secrets::Secrets, setup::Setup};

pub(super) fn forget_replaced(
    secrets: Option<Secrets>,
    before: Option<Setup>,
    after: Option<Setup>,
) -> Result<(), String> {
    let Some(((secrets, before), after)) = secrets.zip(before).zip(after) else {
        return Ok(());
    };
    let kept = scope::token_urls(&after);
    for url in scope::token_urls(&before).difference(&kept) {
        secrets
            .forget(&crate::monitoring::token_account(url))
            .map_err(|_| {
                "The setup was saved, but its replaced token couldn’t be removed from keys.json."
                    .to_owned()
            })?;
    }
    Ok(())
}
