/// revscope.co's billing settings, read only by the private billing module.
const BILLING_ENV_VARS: &[&str] = &[
    "POLAR_CHECKOUT_KEY",
    "POLAR_WEBHOOK_SECRET",
    "STRIPE_SECRET_KEY",
    "STRIPE_WEBHOOK_SECRET",
];

/// Refuses a hosted build without the `billing` feature when billing is
/// configured, since it would give every signup a free, unlimited account and
/// 404 the providers' webhooks. A self-hosted build never bills, and these
/// generic names may reach it from an env file shared with other services.
pub fn require_billing_when_configured() -> Result<(), String> {
    if cfg!(any(feature = "billing", feature = "self-hosted")) {
        return Ok(());
    }
    match BILLING_ENV_VARS
        .iter()
        .find(|name| std::env::var(name).is_ok_and(|value| !value.trim().is_empty()))
    {
        Some(name) => Err(format!(
            "{name} is set but this binary was built without --features billing"
        )),
        None => Ok(()),
    }
}
