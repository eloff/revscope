//! Consent-required jurisdictions and SQL helpers for visitor visibility.
//!
//! Sessions from these locations are hidden from the visitors / journey APIs
//! unless `landings.consent = 1`. Aggregates still include cookieless rows.

/// Comma-separated ISO 3166-1 alpha-2 codes that require analytics consent.
/// Search with `"{CC},"`. Must end in a comma.
pub(crate) const CONSENT_REQUIRED_COUNTRIES: &str = "AT,BE,BG,HR,CH,CY,CZ,DK,EE,FI,FR,DE,GR,HU,IE,IT,LV,LT,LU,MT,NL,PL,PT,RO,SK,SI,ES,SE,IS,LI,NO,GB,";

/// True when the country alone requires consent (EU/EEA/UK/CH list).
pub(crate) fn country_requires_consent(country: Option<&str>) -> bool {
    let Some(country) = non_empty(country) else {
        return false;
    };
    let country = country.to_ascii_uppercase();
    let needle = format!("{country},");
    CONSENT_REQUIRED_COUNTRIES.contains(&needle)
}

/// True when country/region requires analytics consent before persistent
/// attribution. Fail open when country is unknown; Quebec Law 25 only when
/// both CA and QC are present.
pub(crate) fn location_requires_consent(country: Option<&str>, region: Option<&str>) -> bool {
    if country_requires_consent(country) {
        return true;
    }
    matches!(
        (
            non_empty(country).map(|value| value.to_ascii_uppercase()),
            non_empty(region).map(|value| value.to_ascii_uppercase()),
        ),
        (Some(country), Some(region)) if country == "CA" && region == "QC"
    )
}

/// SQL fragment: keep landings with granted consent, or outside consent-required
/// jurisdictions. `country_col` / `region_col` / `consent_col` must be qualified
/// (e.g. `"l.country"`). Builds the country `IN` list from
/// [`CONSENT_REQUIRED_COUNTRIES`].
pub(crate) fn consent_visibility_clause(
    consent_col: &str,
    country_col: &str,
    region_col: &str,
) -> String {
    debug_assert!(
        consent_col.contains('.'),
        "consent_col must be qualified (e.g. \"l.consent\"), got {consent_col:?}"
    );
    debug_assert!(
        country_col.contains('.'),
        "country_col must be qualified (e.g. \"l.country\"), got {country_col:?}"
    );
    debug_assert!(
        region_col.contains('.'),
        "region_col must be qualified (e.g. \"l.region\"), got {region_col:?}"
    );

    let in_list = consent_required_countries_sql_in_list();
    // Brazil (LGPD): hide individual journeys without consent, but do not require
    // consent for cookieless / persistent session policy (not in
    // CONSENT_REQUIRED_COUNTRIES).
    format!(
        " AND ({consent_col} = 1 OR {country_col} IS NULL OR (
            {country_col} NOT IN ({in_list})
            AND {country_col} != 'BR'
            AND NOT ({country_col} = 'CA' AND {region_col} = 'QC')
        ))"
    )
}

/// Quoted `'AT','BE',...` list derived from [`CONSENT_REQUIRED_COUNTRIES`].
fn consent_required_countries_sql_in_list() -> String {
    CONSENT_REQUIRED_COUNTRIES
        .split(',')
        .filter(|code| !code.is_empty())
        .map(|code| format!("'{code}'"))
        .collect::<Vec<_>>()
        .join(",")
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn country_list_drives_requires_consent() {
        assert!(country_requires_consent(Some("DE")));
        assert!(country_requires_consent(Some("gb")));
        assert!(!country_requires_consent(Some("BR")));
        assert!(!country_requires_consent(Some("US")));
        assert!(!country_requires_consent(Some("CA")));
        assert!(!country_requires_consent(None));
    }

    #[test]
    fn quebec_requires_consent_only_with_region() {
        assert!(location_requires_consent(Some("CA"), Some("QC")));
        assert!(location_requires_consent(Some("ca"), Some("qc")));
        assert!(!location_requires_consent(Some("CA"), Some("ON")));
        assert!(!location_requires_consent(Some("CA"), None));
        assert!(!location_requires_consent(None, Some("QC")));
    }

    #[test]
    fn visibility_clause_uses_constant_countries() {
        let clause = consent_visibility_clause("l.consent", "l.country", "l.region");
        assert!(clause.contains("'DE'"));
        assert!(clause.contains("'GB'"));
        assert!(clause.contains("l.country != 'BR'"));
        assert!(!CONSENT_REQUIRED_COUNTRIES.contains("BR,"));
        assert!(clause.contains("l.consent = 1"));
        assert!(clause.contains("l.country = 'CA' AND l.region = 'QC'"));
        // Every code from the constant appears once in the IN list.
        for code in CONSENT_REQUIRED_COUNTRIES
            .split(',')
            .filter(|c| !c.is_empty())
        {
            assert!(
                clause.contains(&format!("'{code}'")),
                "missing {code} in {clause}"
            );
        }
    }
}
