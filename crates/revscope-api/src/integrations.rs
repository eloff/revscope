//! The integrations a project can connect, and what each one takes.
//!
//! `web/app/src/lib/settingsApi.ts` is the dashboard's form for the same list, and
//! migrations-main holds the `integration_type` enum. Tests on each side compare
//! against this one: the test below reads the TypeScript, and a server e2e test
//! saves every type here through the database.

use serde::Serialize;

/// Which `integrations` column a value is saved to on the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Column {
    /// Stored encrypted.
    ApiKey,
    /// Stored encrypted.
    Secret,
    /// The account the credential belongs to, such as a shop domain. Not encrypted.
    ExternalAccountId,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Field {
    pub column: Column,
    pub label: &'static str,
    pub example: Option<&'static str>,
    pub help: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Integration {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub fields: &'static [Field],
}

impl Integration {
    pub fn field(&self, column: Column) -> Option<&'static Field> {
        self.fields.iter().find(|field| field.column == column)
    }
}

pub const STRIPE: &str = "stripe";
pub const SHOPIFY: &str = "shopify";
pub const SQUARE: &str = "square";
pub const LEMON_SQUEEZY: &str = "lemon_squeezy";
pub const POLAR: &str = "polar";
pub const DODO: &str = "dodo";
pub const GOOGLE_SEARCH_CONSOLE: &str = "google_search_console";
pub const CLOUDFLARE: &str = "cloudflare";

pub const INTEGRATIONS: &[Integration] = &[
    Integration {
        id: STRIPE,
        label: "Stripe",
        description: "Sync payments, subscriptions, and refunds.",
        fields: &[Field {
            column: Column::ApiKey,
            label: "Secret key",
            example: Some("sk_live_..."),
            help: Some(
                "Prefer a restricted key with read access to Balance, Charges, Checkout Sessions, Coupons, Disputes, Invoices, PaymentIntents, Refunds, and Subscriptions.",
            ),
        }],
    },
    Integration {
        id: SHOPIFY,
        label: "Shopify",
        description: "Sync orders, refunds, and subscription contracts.",
        fields: &[
            Field {
                column: Column::ExternalAccountId,
                label: "Shop domain",
                example: Some("example.myshopify.com"),
                help: Some("Use the store's myshopify.com domain, not a custom domain."),
            },
            Field {
                column: Column::ApiKey,
                label: "Client ID",
                example: None,
                help: Some(
                    "From a Dev Dashboard app installed on the store with read_orders and read_own_subscription_contracts scopes.",
                ),
            },
            Field {
                column: Column::Secret,
                label: "Client secret",
                example: Some("shpss_..."),
                help: None,
            },
        ],
    },
    Integration {
        id: SQUARE,
        label: "Square",
        description: "Sync payments, refunds, subscriptions, and invoices.",
        fields: &[Field {
            column: Column::ApiKey,
            label: "Access token",
            example: None,
            help: Some(
                "Requires PAYMENTS_READ, SUBSCRIPTIONS_READ, INVOICES_READ, MERCHANT_PROFILE_READ, and ITEMS_READ.",
            ),
        }],
    },
    Integration {
        id: LEMON_SQUEEZY,
        label: "Lemon Squeezy",
        description: "Sync subscriptions and subscription invoices.",
        fields: &[Field {
            column: Column::ApiKey,
            label: "API key",
            example: Some("eyJ0eXAiOi..."),
            help: Some(
                "Lemon Squeezy API keys are not scoped; use a live-mode key from Settings, API.",
            ),
        }],
    },
    Integration {
        id: POLAR,
        label: "Polar",
        description: "Sync orders and subscriptions from a Polar organization.",
        fields: &[Field {
            column: Column::ApiKey,
            label: "Organization access token",
            example: Some("polar_oat_..."),
            help: Some("Requires orders:read, subscriptions:read, and refunds:read scopes."),
        }],
    },
    Integration {
        id: DODO,
        label: "Dodo Payments",
        description: "Sync payments, subscriptions, and refunds.",
        fields: &[Field {
            column: Column::ApiKey,
            label: "API key",
            example: None,
            help: Some("Create a read-only key (leave Enable write access unchecked)."),
        }],
    },
    Integration {
        id: GOOGLE_SEARCH_CONSOLE,
        label: "Search Console",
        description: "Attribute organic search traffic, using a Google service account.",
        fields: &[
            Field {
                column: Column::ExternalAccountId,
                label: "Site / property URL",
                example: Some("sc-domain:example.com"),
                help: Some(
                    "The verified Search Console property to read from. Add the service account as a user on this property.",
                ),
            },
            Field {
                column: Column::ApiKey,
                label: "Service account email",
                example: Some("revscope-gsc@example.iam.gserviceaccount.com"),
                help: Some(
                    "Needs Search Console API enabled and Full or Restricted access on the property (read-only is enough).",
                ),
            },
            Field {
                column: Column::Secret,
                label: "Service account private key",
                example: Some("-----BEGIN PRIVATE KEY-----..."),
                help: None,
            },
        ],
    },
    Integration {
        id: CLOUDFLARE,
        label: "Cloudflare",
        description: "Sync AI crawler request counts from Cloudflare GraphQL analytics.",
        fields: &[Field {
            column: Column::ApiKey,
            label: "API token",
            example: None,
            help: Some(
                "Create a token with Zone, Zone, Read and Zone, Analytics, Read. Include every zone that serves your site.",
            ),
        }],
    },
];

pub fn find(id: &str) -> Option<&'static Integration> {
    INTEGRATIONS.iter().find(|integration| integration.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_catalog_lists_the_same_types_and_columns_as_the_dashboard_form() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/app/src/lib/settingsApi.ts");
        let source = std::fs::read_to_string(path).expect("settingsApi.ts");

        let list = source
            .split("INTEGRATION_TYPES = [")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .expect("the INTEGRATION_TYPES array");
        let mut dashboard: Vec<&str> = list
            .split(',')
            .map(|item| item.trim().trim_matches('"'))
            .filter(|item| !item.is_empty())
            .collect();
        dashboard.sort_unstable();
        let mut catalog: Vec<&str> = INTEGRATIONS.iter().map(|item| item.id).collect();
        catalog.sort_unstable();
        assert_eq!(catalog, dashboard);

        // Each form section starts at `<id>: {` and ends where the next one starts.
        for integration in INTEGRATIONS {
            let section = source
                .split(&format!("  {}: {{\n    type:", integration.id))
                .nth(1)
                .and_then(|rest| rest.split("\n  },\n").next())
                .unwrap_or_else(|| panic!("no form section for {}", integration.id));
            let columns: Vec<&str> = section
                .split("column: \"")
                .skip(1)
                .filter_map(|rest| rest.split('"').next())
                .collect();
            let ours: Vec<&str> = integration
                .fields
                .iter()
                .map(|field| match field.column {
                    Column::ApiKey => "api_key",
                    Column::Secret => "secret",
                    Column::ExternalAccountId => "external_account_id",
                })
                .collect();
            assert_eq!(ours, columns, "{}", integration.id);
        }
    }
}
