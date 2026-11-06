//! Display names for the categorical landing columns (`device`, `os`,
//! `browser_family`), shared by every breakdown that shows them.

pub(crate) fn device_name(device: &str) -> String {
    match device {
        "desktop" => "Desktop",
        "desktop_touch" => "Desktop (touch)",
        "mobile" => "Mobile",
        "tablet" => "Tablet",
        other => return capitalized_or_unknown(other),
    }
    .to_string()
}

pub(crate) fn os_name(os: &str) -> String {
    match os {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        "android" => "Android",
        "ios" => "iOS",
        "chromeos" => "ChromeOS",
        "kaios" => "KaiOS",
        "unknown" => "Unknown",
        other => return capitalized_or_unknown(other),
    }
    .to_string()
}

pub(crate) fn browser_family_name(browser_family: &str) -> String {
    match browser_family {
        "chrome" => "Chrome",
        "firefox" => "Firefox",
        "safari" => "Safari",
        "edge" => "Edge",
        "ie" => "Internet Explorer",
        "opera" => "Opera",
        "brave" => "Brave",
        "duckduckgo" => "DuckDuckGo",
        "samsung" => "Samsung Internet",
        "uc" => "UC Browser",
        "qq" => "QQ Browser",
        "baidu" => "Baidu Browser",
        "yandex" => "Yandex Browser",
        "kai" => "KaiOS Browser",
        "other" => "Other",
        "unknown" => "Unknown",
        other => return capitalized_or_unknown(other),
    }
    .to_string()
}

fn capitalized_or_unknown(slug: &str) -> String {
    let mut chars = slug.chars();
    let Some(first) = chars.next() else {
        return "Unknown".to_string();
    };
    first.to_uppercase().chain(chars).collect()
}
