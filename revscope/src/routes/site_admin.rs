//! The site admin's view of every account on the server.

use crate::authorize::SiteAdmin;
use crate::delete_account::{self, DeleteAccountRequest};
use crate::site_admin_accounts;

use super::request::{ApiRequest, RouteResult};

pub(super) fn accounts(request: &ApiRequest, admin: &SiteAdmin) -> RouteResult {
    let offset = request.query(site_admin_accounts::parse_accounts_query)?;
    let outcome = site_admin_accounts::list_accounts(request.context(), admin, offset);
    request.session_json(admin, outcome, "site_admin_accounts")
}

pub(super) fn delete_account(request: &ApiRequest, admin: &SiteAdmin) -> RouteResult {
    let payload = request.json_body::<DeleteAccountRequest>("invalid delete payload")?;
    let outcome = delete_account::delete_account(request.context(), admin, payload);
    request.session_json(admin, outcome, "delete_account")
}
