use serde_json::Value;

use super::rows::{EngagementRow, LandingContextRow, LandingPageView, PageView};
use crate::referrer::ReferrerType;

/// Mirrors `landings` CHECK constraints (and `campaigns.campaign` length) for
/// values generated at enqueue time.
pub(super) fn validate_landing_constraints(landing: &LandingPageView) -> Result<(), String> {
    if let Some(os_version) = landing.ua_context.os_version
        && os_version.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
    {
        return Err(format!(
            "landings.os_version must be NULL or > 0 (got {os_version})"
        ));
    }
    if let Some(language) = landing.language.as_deref()
        && !is_uppercase_iso639_1(language)
    {
        return Err(format!(
            "landings.language must be NULL or a 2-letter uppercase ISO 639-1 code (got {language:?})"
        ));
    }
    if let Some(country) = landing.country.as_deref()
        && !is_uppercase_iso3166_1_alpha2(country)
    {
        return Err(format!(
            "landings.country must be NULL or a 2-letter uppercase ISO 3166-1 alpha-2 code (got {country:?})"
        ));
    }
    if landing.is_google && landing.referrer_type != Some(ReferrerType::Search) {
        return Err(
            "landings.is_google requires referrer_type = 'search' when is_google is set"
                .to_string(),
        );
    }
    if let Some(campaign) = landing.landing_campaign.as_ref()
        && campaign.campaign.is_empty()
    {
        return Err("campaigns.campaign must have length > 0".to_string());
    }
    Ok(())
}

/// Mirrors the `landings.language` CHECK for the follow-up context hit.
pub(super) fn validate_landing_context_constraints(
    context: &LandingContextRow,
) -> Result<(), String> {
    if let Some(language) = context.language.as_deref()
        && !is_uppercase_iso639_1(language)
    {
        return Err(format!(
            "landings.language must be NULL or a 2-letter uppercase ISO 639-1 code (got {language:?})"
        ));
    }
    Ok(())
}

/// Mirrors `pageviews.scrolled_percent` and `metadata.key` CHECKs.
pub(super) fn validate_pageview_constraints(page: &PageView) -> Result<(), String> {
    if let Some(scrolled_percent) = page.scrolled_percent
        && !(0..=100).contains(&scrolled_percent)
    {
        return Err(format!(
            "pageviews.scrolled_percent must be NULL or between 0 and 100 (got {scrolled_percent})"
        ));
    }
    if let Some(metadata_json) = page.metadata_json.as_deref() {
        validate_metadata_json_constraints(metadata_json)?;
    }
    Ok(())
}

/// Mirrors the `events.metadata` CHECK.
pub(super) fn validate_event_metadata(metadata: Option<&str>) -> Result<(), String> {
    if let Some(metadata) = metadata
        && serde_json::from_str::<Value>(metadata).is_err()
    {
        return Err("events.metadata must be NULL or valid JSON".to_string());
    }
    Ok(())
}

pub(super) fn validate_engagement_constraints(engagement: &EngagementRow) -> Result<(), String> {
    if let Some(scrolled_percent) = engagement.scrolled_percent
        && !(0..=100).contains(&scrolled_percent)
    {
        return Err(format!(
            "pageviews.scrolled_percent must be NULL or between 0 and 100 (got {scrolled_percent})"
        ));
    }
    Ok(())
}

pub(super) fn validate_metadata_json_constraints(metadata_json: &str) -> Result<(), String> {
    let parsed: Value = serde_json::from_str(metadata_json)
        .map_err(|_| "metadata must be valid JSON".to_string())?;
    let Value::Object(map) = parsed else {
        return Ok(());
    };
    for key in map.keys() {
        if key.is_empty() {
            return Err("metadata.key must have length > 0".to_string());
        }
    }
    Ok(())
}

fn is_uppercase_iso639_1(value: &str) -> bool {
    value.len() == 2 && value.bytes().all(|b| b.is_ascii_uppercase())
}

fn is_uppercase_iso3166_1_alpha2(value: &str) -> bool {
    is_uppercase_iso639_1(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::attribution::LandingCampaignTuple;
    use crate::ingest::test_support::{sample_landing_pageview, sample_pageview};
    use compact_str::CompactString;

    #[test]
    fn validate_landing_context_constraints_rejects_unnormalized_language() {
        let context = LandingContextRow {
            visitor_timestamp_id: 1_000,
            timezone: Some(CompactString::new("America/Denver")),
            language: Some(CompactString::new("en-US")),
            screen_dimensions: None,
            touch: false,
        };
        let error =
            validate_landing_context_constraints(&context).expect_err("unnormalized language");
        assert!(error.contains("language"), "{error}");
    }

    #[test]
    fn validate_landing_constraints_rejects_os_version_non_positive() {
        let mut landing = sample_landing_pageview(1_000);
        landing.ua_context.os_version = Some(0.0);
        let err = validate_landing_constraints(&landing).expect_err("os_version 0");
        assert!(err.contains("os_version"), "{err}");
    }

    #[test]
    fn validate_landing_constraints_rejects_invalid_language_and_country() {
        let mut landing = sample_landing_pageview(1_000);
        landing.language = Some(CompactString::new("en"));
        let err = validate_landing_constraints(&landing).expect_err("lowercase language");
        assert!(err.contains("language"), "{err}");

        landing.language = Some(CompactString::new("EN"));
        landing.country = Some("us".to_string());
        let err = validate_landing_constraints(&landing).expect_err("lowercase country");
        assert!(err.contains("country"), "{err}");
    }

    #[test]
    fn validate_landing_constraints_rejects_is_google_without_search() {
        let mut landing = sample_landing_pageview(1_000);
        landing.is_google = true;
        landing.referrer_type = Some(ReferrerType::Campaign);
        let err = validate_landing_constraints(&landing).expect_err("is_google");
        assert!(err.contains("is_google"), "{err}");
    }

    #[test]
    fn validate_landing_constraints_rejects_empty_campaign_name() {
        let mut landing = sample_landing_pageview(1_000);
        landing.landing_campaign = Some(LandingCampaignTuple {
            source: "google".to_string(),
            medium: "cpc".to_string(),
            campaign: String::new(),
            click_id: None,
            network: None,
        });
        let err = validate_landing_constraints(&landing).expect_err("empty campaign");
        assert!(err.contains("campaign"), "{err}");
    }

    #[test]
    fn validate_pageview_constraints_rejects_empty_metadata_key() {
        let mut page = sample_pageview(1_000, 1_000, 1);
        page.metadata_json = Some(r#"{"":"x"}"#.to_string());
        let err = validate_pageview_constraints(&page).expect_err("empty metadata key");
        assert!(err.contains("metadata.key"), "{err}");
    }

    #[test]
    fn validate_pageview_constraints_rejects_scrolled_percent_out_of_range() {
        let mut page = sample_pageview(1_000, 1_000, 1);
        page.scrolled_percent = Some(101);
        let err = validate_pageview_constraints(&page).expect_err("scrolled");
        assert!(err.contains("scrolled_percent"), "{err}");
    }

    #[test]
    fn validate_event_metadata_rejects_invalid_metadata_json() {
        let err = validate_event_metadata(Some("{")).expect_err("invalid json");
        assert!(err.contains("metadata"), "{err}");
    }
}
