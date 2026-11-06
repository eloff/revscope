// Lives in the private billing repository at src/billing.
#![cfg(feature = "billing")]
include!("../src/billing/tests/checkout_e2e.rs");
