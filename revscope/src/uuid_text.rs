//! The canonical hyphenated UUID text form, shared by every place that reads or
//! writes one outside Postgres.

const GROUP_LENGTHS: [usize; 5] = [8, 4, 4, 4, 12];

/// `value` as `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` in lowercase hex.
pub(crate) fn format_uuid(value: u128) -> String {
    let hex = format!("{value:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// Reads exactly the hyphenated form, in either case. Checking the shape first
/// also keeps a malformed id from reaching a `::uuid` cast, which would come
/// back as a database error rather than as an unknown id.
pub(crate) fn parse_hyphenated_uuid(value: &str) -> Option<u128> {
    let mut parts = value.split('-');
    for expected in GROUP_LENGTHS {
        let part = parts.next()?;
        if part.len() != expected || !part.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
    }
    if parts.next().is_some() {
        return None;
    }
    u128::from_str_radix(&value.replace('-', ""), 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "0198f0a1-2b3c-7d4e-8f90-1a2b3c4d5e6f";

    #[test]
    fn a_uuid_round_trips_through_its_text_form() {
        let value = parse_hyphenated_uuid(UUID).expect("parse");
        assert_eq!(value, 0x0198f0a1_2b3c_7d4e_8f90_1a2b3c4d5e6f);
        assert_eq!(format_uuid(value), UUID);
        assert_eq!(format_uuid(0), "00000000-0000-0000-0000-000000000000");
        assert_eq!(
            parse_hyphenated_uuid(&UUID.to_ascii_uppercase()),
            Some(value)
        );
    }

    #[test]
    fn only_the_hyphenated_form_parses() {
        assert_eq!(parse_hyphenated_uuid(""), None);
        assert_eq!(parse_hyphenated_uuid("not-a-uuid"), None);
        assert_eq!(parse_hyphenated_uuid(&UUID.replace('-', "")), None);
        assert_eq!(parse_hyphenated_uuid(&format!("{UUID}-extra")), None);
        assert_eq!(parse_hyphenated_uuid(&format!(" {UUID}")), None);
        assert_eq!(
            parse_hyphenated_uuid("0198f0a12-b3c-7d4e-8f90-1a2b3c4d5e6f"),
            None
        );
        assert_eq!(
            parse_hyphenated_uuid("0198f0a1'; DROP TABLE oauth_clients--"),
            None
        );
        assert_eq!(
            parse_hyphenated_uuid("+198f0a1-2b3c-7d4e-8f90-1a2b3c4d5e6f"),
            None
        );
    }
}
