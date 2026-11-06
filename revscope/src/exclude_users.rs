use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use log::warn;

use crate::db::{DbError, ProjectCache, open_main_database_read};
use crate::server_context::ServerContext;
use crate::uuid_text::format_uuid;

const CACHE_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExcludedUser {
    network_address: IpAddr,
    prefix_length: u8,
}

impl ExcludedUser {
    fn new(network_address: IpAddr, prefix_length: u8) -> Option<Self> {
        valid_prefix_length(network_address, prefix_length).then_some(Self {
            network_address: mask_ip(network_address, prefix_length),
            prefix_length,
        })
    }

    fn matches(&self, ip_address: IpAddr) -> bool {
        same_network(ip_address, self.network_address, self.prefix_length)
    }
}

/// TTL-cached account-scoped settings loaded from the main database.
///
/// Stored on [`ProjectCache`]; shared across requests for that project until
/// `expires_at`. Additional account settings can be added here later.
#[derive(Clone, Debug)]
pub(crate) struct AccountSettingsCache {
    excluded_users: Arc<[ExcludedUser]>,
    expires_at: Instant,
}

impl AccountSettingsCache {
    fn new(excluded_users: Vec<ExcludedUser>) -> Self {
        Self {
            excluded_users: excluded_users.into(),
            expires_at: Instant::now() + CACHE_TTL,
        }
    }

    pub(crate) fn is_fresh(&self) -> bool {
        Instant::now() < self.expires_at
    }

    fn excluded_users(&self) -> &[ExcludedUser] {
        &self.excluded_users
    }
}

pub(crate) async fn should_exclude_ingest(
    server_context: Arc<ServerContext>,
    project: &ProjectCache,
    ip_address: IpAddr,
) -> bool {
    let settings = match account_settings_for_project(server_context, project).await {
        Ok(Some(settings)) => settings,
        Ok(None) => return false,
        Err(error) => {
            // Exclusion filtering is ancillary. Fail open so a temporary main
            // database problem does not cause permanent analytics data loss.
            warn!(
                "failed to load ingest exclusions for project {}: {error}",
                project.id()
            );
            return false;
        }
    };

    settings
        .excluded_users()
        .iter()
        .any(|excluded| excluded.matches(ip_address))
}

async fn account_settings_for_project(
    server_context: Arc<ServerContext>,
    project: &ProjectCache,
) -> Result<Option<AccountSettingsCache>, DbError> {
    if let Some(settings) = project.account_settings_if_fresh() {
        return Ok(Some(settings));
    }
    load_account_settings(server_context, project)
        .await
        .map(Some)
}

async fn load_account_settings(
    server_context: Arc<ServerContext>,
    project: &ProjectCache,
) -> Result<AccountSettingsCache, DbError> {
    let account_uuid = format_uuid(project.account_id());
    let conn = open_main_database_read(server_context).await?;
    let rows = conn
        .query(
            "SELECT network::text
             FROM exclude_users
             WHERE account_id = $1::text::uuid
               AND expires_at > NOW()",
            &[&account_uuid],
        )
        .await
        .map_err(DbError::main_postgres_query)?;
    drop(conn);

    let excluded_users = rows
        .iter()
        .filter_map(|row| excluded_user_from_values(row.get::<_, String>(0)))
        .collect::<Vec<_>>();

    let settings = AccountSettingsCache::new(excluded_users);
    project.store_account_settings(settings.clone());
    Ok(settings)
}

fn excluded_user_from_values(network: String) -> Option<ExcludedUser> {
    let (network_address, prefix_length) = parse_cidr(&network)?;
    ExcludedUser::new(network_address, prefix_length)
}

fn parse_cidr(network: &str) -> Option<(IpAddr, u8)> {
    let (address, prefix_length) = network.rsplit_once('/')?;
    let address = address.parse().ok()?;
    let prefix_length = prefix_length.parse().ok()?;
    valid_prefix_length(address, prefix_length).then_some((address, prefix_length))
}

fn valid_prefix_length(ip_address: IpAddr, prefix_length: u8) -> bool {
    match ip_address {
        IpAddr::V4(_) => prefix_length <= 32,
        IpAddr::V6(_) => prefix_length <= 128,
    }
}

fn same_network(left: IpAddr, right: IpAddr, prefix_length: u8) -> bool {
    match (left, right) {
        (IpAddr::V4(left), IpAddr::V4(right)) if prefix_length <= 32 => {
            mask_ipv4(left, prefix_length) == mask_ipv4(right, prefix_length)
        }
        (IpAddr::V6(left), IpAddr::V6(right)) if prefix_length <= 128 => {
            mask_ipv6(left, prefix_length) == mask_ipv6(right, prefix_length)
        }
        _ => false,
    }
}

fn mask_ip(ip_address: IpAddr, prefix_length: u8) -> IpAddr {
    match ip_address {
        IpAddr::V4(ip_address) => IpAddr::V4(mask_ipv4(ip_address, prefix_length)),
        IpAddr::V6(ip_address) => IpAddr::V6(mask_ipv6(ip_address, prefix_length)),
    }
}

fn mask_ipv4(ip_address: Ipv4Addr, prefix_length: u8) -> Ipv4Addr {
    let mask = if prefix_length == 0 {
        0
    } else {
        u32::MAX << (32 - prefix_length)
    };
    Ipv4Addr::from(u32::from(ip_address) & mask)
}

fn mask_ipv6(ip_address: Ipv6Addr, prefix_length: u8) -> Ipv6Addr {
    let mask = if prefix_length == 0 {
        0
    } else {
        u128::MAX << (128 - prefix_length)
    };
    Ipv6Addr::from(u128::from(ip_address) & mask)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::insert_project;

    #[test]
    fn parses_cidr_text() {
        assert_eq!(
            parse_cidr("203.0.113.0/24"),
            Some(("203.0.113.0".parse().unwrap(), 24))
        );
        assert_eq!(
            parse_cidr("2001:db8:1234:5678::/64"),
            Some(("2001:db8:1234:5678::".parse().unwrap(), 64))
        );
        assert_eq!(parse_cidr("203.0.113.0"), None);
        assert_eq!(parse_cidr("203.0.113.0/99"), None);
    }

    #[test]
    fn matches_address_prefix() {
        let excluded = ExcludedUser::new("203.0.113.0".parse().unwrap(), 24).unwrap();

        assert!(excluded.matches("203.0.113.255".parse().unwrap()));
        assert!(excluded.matches("203.0.113.10".parse().unwrap()));
        assert!(!excluded.matches("203.0.114.10".parse().unwrap()));
        assert!(!excluded.matches("2001:db8::1".parse().unwrap()));
    }

    #[tokio::test]
    async fn matches_fresh_account_settings_cache_without_database_access() {
        let context = Arc::new(ServerContext::new_in_memory("exclude-users-cache-test"));
        let account_key = 42;
        let project = insert_project(context.as_ref(), 1, "demo".to_string(), account_key);
        project.store_account_settings(AccountSettingsCache::new(vec![
            ExcludedUser::new("198.51.100.0".parse().unwrap(), 24).unwrap(),
        ]));

        assert!(
            should_exclude_ingest(context.clone(), &project, "198.51.100.99".parse().unwrap())
                .await
        );
        assert!(!should_exclude_ingest(context, &project, "198.51.101.99".parse().unwrap()).await);
    }
}
