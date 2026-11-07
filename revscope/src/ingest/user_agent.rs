use serde::Serialize;
use woothee::parser::Parser as UserAgentParser;

use super::wire::TrackRequest;
use crate::crawlers::classify as classify_crawler;

/// Longest `User-Agent` we keep. Real browsers stay well under this; the cap
/// bounds what a bot can push into `useragents.useragent`.
const MAX_USER_AGENT_LEN: usize = 512;

/// Text that browsers don't send. Bots often append these to a real browser's
/// UA, so they count even when the parser recognizes the browser. The `/` ends
/// a product name, which keeps a CUBOT_X50 phone or a SpiderSolitaire app's web
/// view from matching.
const BOT_MARKERS: &[&str] = &["bot/", "crawler/", "spider/", "+http"];

/// Variants are spelled so that the serde `snake_case` name is exactly the value
/// [`BrowserFamily::as_str`] stores in `landings.browser_family`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BrowserFamily {
    Unknown = 0,
    Chrome = 1,
    Firefox = 2,
    Safari = 3,
    Edge = 4,
    Ie = 5,
    Opera = 6,
    Brave = 7,
    Duckduckgo = 8,
    Samsung = 9,
    Uc = 10,
    Qq = 11,
    Baidu = 12,
    Yandex = 13,
    Kai = 14,
    /// Any browser without a dedicated family, including ones Woothee doesn't
    /// recognize: Webview, Vivaldi, Sleipnir, an iPad app's web view.
    Other = 15,
}

impl BrowserFamily {
    pub(super) fn as_str(self) -> Option<&'static str> {
        match self {
            Self::Unknown => None,
            Self::Chrome => Some("chrome"),
            Self::Firefox => Some("firefox"),
            Self::Safari => Some("safari"),
            Self::Edge => Some("edge"),
            Self::Ie => Some("ie"),
            Self::Opera => Some("opera"),
            Self::Brave => Some("brave"),
            Self::Duckduckgo => Some("duckduckgo"),
            Self::Samsung => Some("samsung"),
            Self::Uc => Some("uc"),
            Self::Qq => Some("qq"),
            Self::Baidu => Some("baidu"),
            Self::Yandex => Some("yandex"),
            Self::Kai => Some("kai"),
            Self::Other => Some("other"),
        }
    }
}

/// Variants are spelled so that the serde `snake_case` name is exactly the value
/// [`OperatingSystem::as_str`] stores in `landings.os`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum OperatingSystem {
    Unknown = 0,
    Windows = 1,
    Macos = 2,
    Linux = 3,
    Android = 4,
    Ios = 5,
    Chromeos = 6,
    Kaios = 7,
}

impl OperatingSystem {
    pub(super) fn as_str(self) -> Option<&'static str> {
        match self {
            Self::Unknown => None,
            Self::Windows => Some("windows"),
            Self::Macos => Some("macos"),
            Self::Linux => Some("linux"),
            Self::Android => Some("android"),
            Self::Ios => Some("ios"),
            Self::Chromeos => Some("chromeos"),
            Self::Kaios => Some("kaios"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DeviceClass {
    Desktop,
    DesktopTouch,
    Mobile,
    Tablet,
}

impl DeviceClass {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::DesktopTouch => "desktop_touch",
            Self::Mobile => "mobile",
            Self::Tablet => "tablet",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct UserAgentContext {
    pub(super) browser_family: BrowserFamily,
    pub(super) os: OperatingSystem,
    pub(super) os_version: Option<f64>,
    pub(super) device: DeviceClass,
    /// Bot hits go to `crawler_views_by_hour` instead of `landings`.
    #[serde(skip)]
    pub(super) is_bot: bool,
    /// Lower-cased user agent, kept only for bots.
    pub(super) useragent: Option<String>,
}

/// Lower-cased `User-Agent` header, truncated to [`MAX_USER_AGENT_LEN`], as we
/// store and compare it.
pub(super) fn normalized_user_agent(user_agent: Option<&str>) -> Option<String> {
    let user_agent = user_agent?.trim();
    if user_agent.is_empty() {
        return None;
    }
    let mut end = user_agent.len().min(MAX_USER_AGENT_LEN);
    while !user_agent.is_char_boundary(end) {
        end -= 1;
    }
    Some(user_agent[..end].to_ascii_lowercase())
}

/// `user_agent` must be the header as sent: woothee matches browser and OS
/// names case-sensitively.
pub(super) fn parse_user_agent_context(
    user_agent: Option<&str>,
    payload: &TrackRequest,
) -> UserAgentContext {
    let parser = UserAgentParser::new();
    let mut browser_family = BrowserFamily::Unknown;
    let mut os = OperatingSystem::Unknown;
    let mut os_version = None;
    let mut user_agent_category: Option<String> = None;

    if let Some(ua) = user_agent
        && let Some(parsed) = parser.parse(ua)
    {
        browser_family = map_browser_family(parsed.name, payload);
        // A browser Woothee knows but we have no family for: Webview, Vivaldi, …
        if browser_family == BrowserFamily::Unknown && parsed.browser_type == "browser" {
            browser_family = BrowserFamily::Other;
        }
        os = map_operating_system(parsed.os, payload);
        os_version = parse_version_number(&parsed.os_version);
        user_agent_category = Some(parsed.category.to_ascii_lowercase());
    }

    if browser_family == BrowserFamily::Unknown {
        browser_family = map_browser_family("", payload);
    }
    if os == OperatingSystem::Unknown {
        os = map_operating_system("", payload);
    }

    let normalized = normalized_user_agent(user_agent);
    let device = resolve_device_class(
        normalized.as_deref(),
        payload,
        user_agent_category.as_deref(),
    );
    let is_bot = has_bot_evidence(
        normalized.as_deref(),
        user_agent_category.as_deref(),
        browser_family != BrowserFamily::Unknown,
    );
    // Calling a person a bot loses their session, events and revenue, while the
    // reverse adds one mostly empty session. So a browser we can't name is still
    // a pageview unless something says otherwise.
    if !is_bot && browser_family == BrowserFamily::Unknown {
        browser_family = BrowserFamily::Other;
    }
    let useragent = is_bot.then_some(normalized).flatten();

    UserAgentContext {
        browser_family,
        os,
        os_version,
        device,
        is_bot,
        useragent,
    }
}

/// `user_agent` must be lower-cased. Every browser sends a user agent, and all
/// but a few old ones Woothee recognizes start with `Mozilla/`.
fn has_bot_evidence(
    user_agent: Option<&str>,
    ua_category: Option<&str>,
    recognized_browser: bool,
) -> bool {
    let Some(ua) = user_agent else {
        return true;
    };
    classify_crawler(ua).is_some()
        || ua_category == Some("crawler")
        || BOT_MARKERS.iter().any(|marker| ua.contains(marker))
        // Otherwise only bots say "compatible;" today.
        || (ua.contains("compatible;") && !ua.contains("msie") && !ua.contains("konqueror"))
        || (!recognized_browser && !ua.starts_with("mozilla/"))
}

/// `user_agent` must be lower-cased, the checks here match on substrings.
fn resolve_device_class(
    user_agent: Option<&str>,
    payload: &TrackRequest,
    ua_category: Option<&str>,
) -> DeviceClass {
    let ua = user_agent.unwrap_or("");
    let category = ua_category.unwrap_or("");

    let is_tablet = category.contains("tablet") || ua.contains("ipad") || ua.contains("tablet");
    if is_tablet {
        return DeviceClass::Tablet;
    }

    let is_mobile = payload.mobile.unwrap_or(false)
        || category.contains("smartphone")
        || category.contains("mobile")
        || (ua.contains("mobile") && !ua.contains("ipad"));
    if is_mobile {
        return DeviceClass::Mobile;
    }

    if payload.touch.unwrap_or(false) {
        return DeviceClass::DesktopTouch;
    }

    DeviceClass::Desktop
}

fn map_browser_family(browser_name: &str, payload: &TrackRequest) -> BrowserFamily {
    if payload.brave.unwrap_or(false) {
        return BrowserFamily::Brave;
    }
    if payload.duck.unwrap_or(false) {
        return BrowserFamily::Duckduckgo;
    }

    let normalized = browser_name.to_ascii_lowercase();
    if normalized.contains("edge") {
        BrowserFamily::Edge
    } else if normalized.contains("firefox") {
        BrowserFamily::Firefox
    } else if normalized.contains("safari") {
        BrowserFamily::Safari
    } else if normalized.contains("msie") || normalized.contains("internet explorer") {
        BrowserFamily::Ie
    } else if normalized.contains("opera") {
        BrowserFamily::Opera
    } else if normalized.contains("samsung") {
        BrowserFamily::Samsung
    } else if normalized.contains("uc browser") || normalized == "uc" {
        BrowserFamily::Uc
    } else if normalized.contains("qq") {
        BrowserFamily::Qq
    } else if normalized.contains("baidu") {
        BrowserFamily::Baidu
    } else if normalized.contains("yandex") {
        BrowserFamily::Yandex
    } else if normalized.contains("kai") {
        BrowserFamily::Kai
    } else if normalized.contains("chrome") {
        BrowserFamily::Chrome
    } else {
        BrowserFamily::Unknown
    }
}

fn map_operating_system(os_name: &str, payload: &TrackRequest) -> OperatingSystem {
    let normalized = os_name.to_ascii_lowercase();
    if normalized.contains("windows") {
        OperatingSystem::Windows
    } else if normalized.contains("android") {
        OperatingSystem::Android
    } else if normalized.contains("iphone")
        || normalized.contains("ipad")
        || normalized.contains("ios")
    {
        OperatingSystem::Ios
    } else if normalized.contains("mac") || normalized.contains("darwin") {
        OperatingSystem::Macos
    } else if normalized.contains("cros") || normalized.contains("chrome os") {
        OperatingSystem::Chromeos
    } else if normalized.contains("kai") {
        OperatingSystem::Kaios
    } else if normalized.contains("linux") {
        OperatingSystem::Linux
    } else if payload.mobile.unwrap_or(false) {
        OperatingSystem::Android
    } else {
        OperatingSystem::Unknown
    }
}

fn parse_version_number(value: &str) -> Option<f64> {
    if value.is_empty() {
        return None;
    }
    let token = value
        .split(|character: char| !(character.is_ascii_digit() || character == '.'))
        .find(|segment| !segment.is_empty())?;
    token.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::test_support::sample_payload;

    #[test]
    fn parse_user_agent_context_maps_known_browser_and_os() {
        let mut payload = sample_payload();
        payload.mobile = Some(false);
        let parsed = parse_user_agent_context(
            Some(
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/123.0.0.0 Safari/537.36",
            ),
            &payload,
        );
        assert_eq!(parsed.browser_family, BrowserFamily::Chrome);
        assert_eq!(parsed.os, OperatingSystem::Linux);
        assert_eq!(parsed.useragent, None);
    }

    #[test]
    fn parse_user_agent_context_keeps_useragent_for_unknown_browsers() {
        let payload = sample_payload();
        let parsed = parse_user_agent_context(
            Some("Mozilla/5.0 (compatible; SomeBot/2.1; +http://bots.example/bot)"),
            &payload,
        );
        assert_eq!(parsed.browser_family, BrowserFamily::Unknown);
        assert_eq!(
            parsed.useragent.as_deref(),
            Some("mozilla/5.0 (compatible; somebot/2.1; +http://bots.example/bot)")
        );

        let without_ua = parse_user_agent_context(None, &payload);
        assert!(without_ua.is_bot);
        assert_eq!(without_ua.useragent, None);
    }

    #[test]
    fn parse_user_agent_context_records_woothee_browsers_as_other() {
        let mut mobile = sample_payload();
        mobile.mobile = Some(true);
        let webview = parse_user_agent_context(
            Some(
                "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 Twitter for iPhone/10.45",
            ),
            &mobile,
        );
        assert_eq!(webview.browser_family, BrowserFamily::Other);
        assert_eq!(webview.os, OperatingSystem::Ios);
        assert_eq!(webview.device, DeviceClass::Mobile);
        assert_eq!(webview.useragent, None);

        let vivaldi = parse_user_agent_context(
            Some(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/48.0.2564.88 Safari/537.36 Vivaldi/1.0.380.2",
            ),
            &sample_payload(),
        );
        assert_eq!(vivaldi.browser_family, BrowserFamily::Other);
        assert_eq!(vivaldi.os, OperatingSystem::Windows);
        assert_eq!(vivaldi.useragent, None);

        let googlebot = parse_user_agent_context(
            Some("Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)"),
            &sample_payload(),
        );
        assert_eq!(googlebot.browser_family, BrowserFamily::Unknown);
        assert!(googlebot.useragent.is_some());
    }

    #[test]
    fn browsers_without_bot_evidence_are_people() {
        for ua in [
            // iPad and macOS app web views: no browser name at all.
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) MyApp/2.1",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) [FBAN/FBIOS;FBAV/450.0;FBDV/iPad13,4]",
            "Mozilla/5.0 (iPad; CPU OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15",
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Ladybird/1.0",
            "Mozilla/5.0 (Linux; Android 11; CUBOT_X50 Build/RP1A) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Mobile Safari/537.36",
            "Mozilla/5.0 (compatible; MSIE 10.0; Windows NT 6.1; Trident/6.0)",
            "Mozilla/5.0 (compatible; Konqueror/4.5; Linux) KHTML/4.5.4 (like Gecko)",
            "Mozilla/5.0 (iPad; CPU OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 SpiderSolitaire/2.0",
            "Opera/9.80 (Windows NT 6.1) Presto/2.12.388 Version/12.18",
        ] {
            let parsed = parse_user_agent_context(Some(ua), &sample_payload());
            assert!(!parsed.is_bot, "{ua}");
            assert_ne!(parsed.browser_family, BrowserFamily::Unknown, "{ua}");
            assert_eq!(parsed.useragent, None, "{ua}");
        }
    }

    #[test]
    fn evidence_of_a_bot_wins_over_a_recognized_browser() {
        for ua in [
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_5) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/13.1.1 Safari/605.1.15 (Applebot/0.1; +http://www.apple.com/go/applebot)",
            "Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; Amazonbot/0.1; +https://developer.amazon.com/support/amazonbot) Chrome/119.0.6045.214 Safari/537.36",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36; compatible; OAI-SearchBot/1.3; +https://openai.com/searchbot",
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/126.0.0.0 Safari/537.36",
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36 (Stripebot/1.0; +https://docs.stripe.com/stripebot-crawler)",
            "Mozilla/5.0 (Linux; Android 6.0.1; Nexus 5X Build/MMB29P) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Mobile Safari/537.36 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)",
            "Mozilla/5.0 (compatible; YandexBot/3.0; +http://yandex.com/bots)",
            "Mozilla/5.0 (compatible; dataprovider.com)",
            "curl/8.5.0",
            "pc",
            "lightpanda/1.0",
            "yisouspider",
            "Mozilla/5.0 (compatible; Baiduspider/2.0; +http://www.baidu.com/search/spider.html)",
            "Mozilla/5.0 (X11; Linux x86_64) Chrome/126.0.0.0 Safari/537.36 SomeCrawler/1.0",
        ] {
            let parsed = parse_user_agent_context(Some(ua), &sample_payload());
            assert!(parsed.is_bot, "{ua}");
            assert_eq!(
                parsed.useragent.as_deref(),
                Some(ua.to_ascii_lowercase().as_str()),
                "{ua}"
            );
        }
    }

    /// The recorded JSON for a dropped row has to show the same device strings we
    /// write to `landings`, so the serde name of every variant must equal its
    /// `as_str()` value. `Unknown` is stored as NULL and serializes as "unknown".
    #[test]
    fn user_agent_enum_serde_names_match_the_stored_column_values() {
        for browser_family in [
            BrowserFamily::Unknown,
            BrowserFamily::Chrome,
            BrowserFamily::Firefox,
            BrowserFamily::Safari,
            BrowserFamily::Edge,
            BrowserFamily::Ie,
            BrowserFamily::Opera,
            BrowserFamily::Brave,
            BrowserFamily::Duckduckgo,
            BrowserFamily::Samsung,
            BrowserFamily::Uc,
            BrowserFamily::Qq,
            BrowserFamily::Baidu,
            BrowserFamily::Yandex,
            BrowserFamily::Kai,
            BrowserFamily::Other,
        ] {
            assert_eq!(
                serde_name(browser_family),
                browser_family.as_str().unwrap_or("unknown")
            );
        }

        for os in [
            OperatingSystem::Unknown,
            OperatingSystem::Windows,
            OperatingSystem::Macos,
            OperatingSystem::Linux,
            OperatingSystem::Android,
            OperatingSystem::Ios,
            OperatingSystem::Chromeos,
            OperatingSystem::Kaios,
        ] {
            assert_eq!(serde_name(os), os.as_str().unwrap_or("unknown"));
        }

        for device in [
            DeviceClass::Desktop,
            DeviceClass::DesktopTouch,
            DeviceClass::Mobile,
            DeviceClass::Tablet,
        ] {
            assert_eq!(serde_name(device), device.as_str());
        }
    }

    fn serde_name(value: impl Serialize) -> String {
        serde_json::to_value(value)
            .expect("serialize variant")
            .as_str()
            .expect("variant serializes as a string")
            .to_string()
    }

    #[test]
    fn normalized_user_agent_lower_cases_trims_and_truncates() {
        assert_eq!(
            normalized_user_agent(Some("  Mozilla/5.0 Test  ")).as_deref(),
            Some("mozilla/5.0 test")
        );
        assert_eq!(normalized_user_agent(Some("   ")), None);
        assert_eq!(normalized_user_agent(None), None);

        let long = "A".repeat(MAX_USER_AGENT_LEN + 10);
        assert_eq!(
            normalized_user_agent(Some(&long)).map(|ua| ua.len()),
            Some(MAX_USER_AGENT_LEN)
        );
        // Truncation must not split a multi-byte character: the cap lands inside
        // one of the two-byte chars, so it backs up a byte.
        let multibyte = format!("x{}", "é".repeat(MAX_USER_AGENT_LEN));
        assert_eq!(
            normalized_user_agent(Some(&multibyte)).map(|ua| ua.len()),
            Some(MAX_USER_AGENT_LEN - 1)
        );
    }
}
