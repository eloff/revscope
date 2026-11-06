use std::net::IpAddr;

#[cfg(geoip_embedded)]
use std::sync::LazyLock;

#[cfg(geoip_embedded)]
use log::{error, warn};
#[cfg(geoip_embedded)]
use maxminddb::{Reader, geoip2};

/// City-level GeoIP database, opened once on first use and shared (read-only)
/// across all requests for the lifetime of the process. `None` if the database
/// could not be opened, in which case lookups degrade to empty results.
///
/// Compiled only when `GeoLite2-City.mmdb` is present next to this crate
/// (`build.rs` sets `geoip_embedded`). Without that file there is no reader
/// and [`resolve`] uses Cloudflare's country header instead.
#[cfg(geoip_embedded)]
static GEOIP_READER: LazyLock<Option<Reader<&'static [u8]>>> = LazyLock::new(|| {
    static DB: &[u8] = include_bytes!("../GeoLite2-City.mmdb");
    match Reader::from_source(DB) {
        Ok(reader) => Some(reader),
        Err(error) => {
            error!("failed to open embedded GeoLite2 city database: {error}");
            None
        }
    }
});

/// Resolved location for a visitor IP. Fields are absent when the database
/// is unavailable, the IP is not in the database, or the lookup fails.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct GeoLocation {
    /// ISO 3166-1 alpha-2 country code (uppercase).
    pub(crate) country: Option<String>,
    /// ISO 3166-2 subdivision code (e.g. province/state), uppercase when present.
    pub(crate) region: Option<String>,
    /// City name (English).
    pub(crate) city: Option<String>,
}

/// Looks up the country, region, and city for a visitor.
///
/// When the GeoLite2 database was embedded at compile time, `ip` is looked up
/// and `cf_country` is ignored. When it was not, no lookup runs: `country` is
/// taken from `cf_country` (Cloudflare's `CF-IPCountry`) and region and city
/// stay empty. The caller must pass that header only for a trusted proxy.
/// Never fails: a missing database, a missing header, or a lookup error yields
/// an empty [`GeoLocation`] so ingest can proceed.
pub(crate) fn resolve(ip: Option<IpAddr>, cf_country: Option<&str>) -> GeoLocation {
    #[cfg(geoip_embedded)]
    {
        let _ = cf_country;
        ip.map(lookup_embedded).unwrap_or_default()
    }
    #[cfg(not(geoip_embedded))]
    {
        let _ = ip;
        GeoLocation {
            country: cloudflare_country(cf_country),
            region: None,
            city: None,
        }
    }
}

/// ISO country from Cloudflare's `CF-IPCountry`. `XX` (unknown) and `T1`
/// (Tor), along with any code this build does not recognize, are absent.
#[cfg_attr(geoip_embedded, allow(dead_code))]
fn cloudflare_country(header: Option<&str>) -> Option<String> {
    let code = header?.trim();
    if code.len() != 2 || !code.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return None;
    }
    let code = code.to_ascii_uppercase();
    if crate::country_region::is_known_country_code(&code) {
        Some(code)
    } else {
        None
    }
}

/// Looks up the country, region, and city for a visitor IP. Never fails: any
/// error (or a missing database) yields an empty `GeoLocation`.
#[cfg(geoip_embedded)]
fn lookup_embedded(ip: IpAddr) -> GeoLocation {
    let Some(reader) = GEOIP_READER.as_ref() else {
        return GeoLocation::default();
    };

    let result = match reader.lookup(ip) {
        Ok(result) => result,
        Err(error) => {
            warn!("geoip lookup failed for {ip}: {error}");
            return GeoLocation::default();
        }
    };

    match result.decode::<geoip2::City>() {
        Ok(Some(record)) => GeoLocation {
            country: record.country.iso_code.map(str::to_string),
            region: record
                .subdivisions
                .first()
                .and_then(|subdivision| subdivision.iso_code)
                .map(str::to_string),
            city: record.city.names.english.map(str::to_string),
        },
        Ok(None) => GeoLocation::default(),
        Err(error) => {
            warn!("geoip decode failed for {ip}: {error}");
            GeoLocation::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloudflare_country_accepts_known_codes() {
        assert_eq!(cloudflare_country(Some("de")).as_deref(), Some("DE"));
        assert_eq!(cloudflare_country(Some(" US ")).as_deref(), Some("US"));
    }

    #[test]
    fn cloudflare_country_rejects_placeholders_and_unknown_codes() {
        assert_eq!(cloudflare_country(None), None);
        assert_eq!(cloudflare_country(Some("")), None);
        assert_eq!(cloudflare_country(Some("XX")), None);
        assert_eq!(cloudflare_country(Some("T1")), None);
        assert_eq!(cloudflare_country(Some("ZZ")), None);
        assert_eq!(cloudflare_country(Some("USA")), None);
    }

    #[test]
    fn resolve_uses_cloudflare_country_only_without_the_embedded_database() {
        let geo = resolve(None, Some("DE"));
        if cfg!(geoip_embedded) {
            assert_eq!(geo, GeoLocation::default());
        } else {
            assert_eq!(geo.country.as_deref(), Some("DE"));
            assert_eq!(geo.region, None);
            assert_eq!(geo.city, None);
        }
    }
}
