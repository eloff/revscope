//! The MCP handler: the read-only tools the stdio `mcp/` package exposes, plus
//! project listing, wired straight to the internal schema and query code
//! instead of looping back through HTTP.
//!
//! The handler itself is stateless. What a call may read arrives with the
//! request: [`transport`](super::transport) puts an [`McpRequest`] in the HTTP
//! extensions, `rmcp` passes the request parts through to the tool, and the tool
//! takes the grant from there. So a tool never has to decide whose data it is
//! allowed to touch, and the service can be built once and shared.
//!
//! A token covers a whole account, or for the demo token just the demo, so
//! `run_query` names its project and every name it is given is checked against
//! the grant before anything is opened.

use std::net::IpAddr;
use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;

use crate::authorize::{AuthorizedProject, authorize_mcp_project};
use crate::download_db::DEMO_PROJECT_ID;
use crate::mcp::token::McpGrant;
use crate::projects::{
    account_project_names, fetch_project, parse_project_id_str, resolve_project,
};
use crate::query::{QueryError, QueryResponse};
use crate::rate_limit::ip_key;
use crate::server_context::ServerContext;

const SERVER_INSTRUCTIONS: &str = "Read-only access to the RevScope analytics databases of one account. Call list_projects for the project names and get_schema for the tables they all share, then run_query against a named project. This server only reads. To create an account, install the tracker on a site, or connect integrations, use the revscope command line tool: https://revscope.co/docs/cli";

/// Everything one MCP request is allowed to do, handed to the tools through the
/// HTTP extensions. The [`McpGrant`] is what the token checked out as; the
/// context is here rather than on the handler so the handler can be shared
/// without holding a reference back to it.
#[derive(Clone)]
pub(crate) struct McpRequest {
    pub(crate) server_context: Arc<ServerContext>,
    pub(crate) grant: McpGrant,
    /// Who is asking, for charging queries on the shared demo to each reader.
    pub(crate) client_ip: Option<IpAddr>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct RunQueryRequest {
    /// Which project to query, spelled exactly as list_projects returns it.
    /// Required: there is no default project.
    // Optional to serde so an omitted name reaches the tool and comes back as a
    // listing of what was available, rather than as a deserialization failure.
    #[schemars(required)]
    project_name: Option<String>,
    /// A single read-only SQL statement.
    sql: String,
    /// Rows to return, at most 500. Defaults to 100.
    #[serde(default)]
    limit: Option<u32>,
    /// Rows to skip, at most 5000. Defaults to 0.
    #[serde(default)]
    offset: Option<u32>,
}

#[derive(Clone)]
pub(crate) struct McpService {
    tool_router: ToolRouter<Self>,
}

impl Default for McpService {
    fn default() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl McpService {
    /// List the RevScope projects on this account. Their names are what
    /// run_query takes, and it takes them exactly as spelled here.
    #[tool(
        name = "list_projects",
        annotations(title = "List RevScope projects", read_only_hint = true)
    )]
    async fn list_projects(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let (server_context, grant) = {
            let request = authorized_request(&context)?;
            (request.server_context.clone(), request.grant.clone())
        };
        let names = project_names(server_context, &grant).await?;

        let listing = if names.is_empty() {
            "this account has no projects yet".to_string()
        } else {
            format!("{} project(s):\n{}", names.len(), names.join("\n"))
        };
        Ok(CallToolResult::success(vec![ContentBlock::text(listing)]))
    }

    /// Return the SQL schema (tables and indices) of a RevScope project's
    /// analytics database, with notes on how to query it. Every project on the
    /// account shares this schema, so it is the same whichever one you query.
    #[tool(
        name = "get_schema",
        annotations(title = "Get RevScope schema", read_only_hint = true)
    )]
    async fn get_schema(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let server_context = authorized_request(&context)?.server_context.clone();
        let dumped = crate::task::spawn_blocking(server_context.clone(), move || {
            crate::schema::cached_schema(server_context)
        })
        .await
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;

        match dumped {
            Ok(schema) => Ok(CallToolResult::success(vec![ContentBlock::text(
                schema.to_string(),
            )])),
            Err(error) => Err(ErrorData::internal_error(
                format!("the schema could not be read: {error:?}"),
                None,
            )),
        }
    }

    /// Run one read-only SQL statement (SELECT, WITH, VALUES, or EXPLAIN) against
    /// the SQLite analytics database of one named RevScope project. Call
    /// list_projects for the names and get_schema to learn the tables.
    /// Paginate with the limit and offset arguments rather than a LIMIT or OFFSET
    /// clause, and always ORDER BY so pages are stable. Blob values are returned
    /// base64 encoded.
    #[tool(
        name = "run_query",
        annotations(title = "Run RevScope query", read_only_hint = true)
    )]
    async fn run_query(
        &self,
        Parameters(RunQueryRequest {
            project_name,
            sql,
            limit,
            offset,
        }): Parameters<RunQueryRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let (server_context, grant, reader) = {
            let request = authorized_request(&context)?;
            let reader = request.client_ip.map(ip_key).unwrap_or_default();
            (
                request.server_context.clone(),
                request.grant.clone(),
                reader,
            )
        };

        let project = match project_name.as_deref() {
            Some(name) => granted_project(server_context.clone(), &grant, name).await?,
            None => None,
        };
        let Some(project) = project else {
            return no_such_project(server_context, &grant, project_name.as_deref()).await;
        };

        let project_id = project.id().to_string();
        let result = crate::task::spawn_blocking(server_context.clone(), move || {
            crate::query::run_project_query(
                server_context,
                &project_id,
                &reader,
                &sql,
                limit,
                offset,
            )
        })
        .await
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;

        match result {
            Ok(response) => Ok(CallToolResult::success(vec![ContentBlock::text(
                format_query_result(&response),
            )])),
            // A statement the caller wrote badly, or one refused while the server
            // is busy, comes back as tool content rather than a protocol error,
            // so the agent can read the message and fix or retry it.
            Err(error @ (QueryError::Invalid(_) | QueryError::Busy)) => Ok(CallToolResult::error(
                vec![ContentBlock::text(error.client_message())],
            )),
            Err(QueryError::Auth(error)) => Err(ErrorData::internal_error(
                format!("the query could not be run: {error:?}"),
                None,
            )),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for McpService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("revscope", env!("CARGO_PKG_VERSION")))
            .with_instructions(SERVER_INSTRUCTIONS)
    }
}

/// The grant the transport attached to this request. Absent only if a tool is
/// reached some other way, which would be a bug rather than a caller's mistake,
/// so it fails closed instead of guessing an account.
fn authorized_request(context: &RequestContext<RoleServer>) -> Result<&McpRequest, ErrorData> {
    context
        .extensions
        .get::<http::request::Parts>()
        .and_then(|parts| parts.extensions.get::<McpRequest>())
        .ok_or_else(|| {
            ErrorData::internal_error("the request carries no account authorization", None)
        })
}

/// Resolves a name a tool call supplied, to the project it names if the grant
/// reaches it. A name the grant does not reach is `None`, the same as one that
/// does not exist anywhere: the answer must not tell the caller which.
async fn granted_project(
    server_context: Arc<ServerContext>,
    grant: &McpGrant,
    project_name: &str,
) -> Result<Option<AuthorizedProject>, ErrorData> {
    let project = resolve_project(server_context, project_name)
        .await
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    Ok(project.and_then(|project| authorize_mcp_project(grant, &project).ok()))
}

/// What a call that named no project, or one this account cannot see, gets
/// back. Tool content rather than a protocol error, with the names it could
/// have used, so the agent can fix the call itself instead of giving up.
async fn no_such_project(
    server_context: Arc<ServerContext>,
    grant: &McpGrant,
    project_name: Option<&str>,
) -> Result<CallToolResult, ErrorData> {
    let names = project_names(server_context, grant).await?;
    let available = if names.is_empty() {
        "this account has no projects yet".to_string()
    } else {
        format!("available projects are: {}", names.join(", "))
    };
    let missing = match project_name {
        Some(project_name) => format!("unknown project name {project_name:?}"),
        None => "missing project name".to_string(),
    };
    Ok(CallToolResult::error(vec![ContentBlock::text(format!(
        "{missing}; {available}"
    ))]))
}

async fn project_names(
    server_context: Arc<ServerContext>,
    grant: &McpGrant,
) -> Result<Vec<String>, ErrorData> {
    let names = match grant {
        McpGrant::Account(account) => account_project_names(server_context, account.id()).await,
        McpGrant::Demo => {
            let demo_id = parse_project_id_str(DEMO_PROJECT_ID)
                .ok_or_else(|| ErrorData::internal_error("the demo project id is invalid", None))?;
            fetch_project(server_context, demo_id).await.map(|demo| {
                demo.into_iter()
                    .filter(|demo| authorize_mcp_project(grant, demo).is_ok())
                    .map(|demo| demo.name().to_string())
                    .collect()
            })
        }
    };
    names.map_err(|error| ErrorData::internal_error(error.to_string(), None))
}

/// Rows as JSON, prefixed with what was actually applied: the agent needs to
/// know when its limit was clamped, and whether another page is waiting. Same
/// shape the stdio package prints, so a prompt written against one reads the
/// other.
fn format_query_result(response: &QueryResponse) -> String {
    let more = if response.has_more {
        format!(
            ", more rows available (next offset {})",
            response.offset as usize + response.rows.len()
        )
    } else {
        String::new()
    };
    let body = serde_json::to_string_pretty(&serde_json::json!({
        "columns": response.columns,
        "rows": response.rows,
    }))
    .unwrap_or_else(|_| "{}".to_string());
    format!(
        "{} row(s), limit {}, offset {}{more}\n{body}",
        response.rows.len(),
        response.limit,
        response.offset,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn the_result_summary_reports_clamping_and_the_next_page() {
        let response = QueryResponse {
            columns: vec!["path".to_string()],
            rows: vec![vec![json!("/pricing")], vec![json!("/signup")]],
            limit: 500,
            offset: 4,
            has_more: true,
        };
        let formatted = format_query_result(&response);
        let (summary, body) = formatted.split_once('\n').expect("summary line");
        assert_eq!(
            summary,
            "2 row(s), limit 500, offset 4, more rows available (next offset 6)"
        );
        let parsed: Value = serde_json::from_str(body).expect("json body");
        assert_eq!(parsed["columns"], json!(["path"]));
        assert_eq!(parsed["rows"], json!([["/pricing"], ["/signup"]]));
    }

    #[test]
    fn a_final_page_does_not_advertise_another() {
        let response = QueryResponse {
            columns: vec!["n".to_string()],
            rows: vec![vec![json!(1)]],
            limit: 100,
            offset: 0,
            has_more: false,
        };
        assert!(
            format_query_result(&response).starts_with("1 row(s), limit 100, offset 0\n"),
            "{}",
            format_query_result(&response)
        );
    }

    #[test]
    fn every_tool_is_advertised_read_only_with_a_description() {
        let router = McpService::tool_router();
        let mut names: Vec<String> = router
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec!["get_schema", "list_projects", "run_query"]);

        for tool in router.list_all() {
            assert_eq!(
                tool.annotations
                    .as_ref()
                    .and_then(|annotations| annotations.read_only_hint),
                Some(true),
                "{} must be marked read-only",
                tool.name
            );
            assert!(
                tool.description
                    .as_deref()
                    .is_some_and(|description| description.contains("RevScope")),
                "{} must describe itself",
                tool.name
            );
        }
    }

    /// The name is `Option` in Rust so an omitted one reaches the tool and gets
    /// the listing back, but a client reading the schema must still be told it
    /// has to send one.
    #[test]
    fn run_query_advertises_the_project_name_as_required() {
        let router = McpService::tool_router();
        let tool = router
            .list_all()
            .into_iter()
            .find(|tool| tool.name == "run_query")
            .expect("run_query");
        let required = tool.input_schema["required"]
            .as_array()
            .expect("required list");
        assert!(required.contains(&json!("project_name")), "{required:?}");
        assert!(required.contains(&json!("sql")), "{required:?}");
    }

    #[test]
    fn an_omitted_project_name_still_deserializes_so_the_tool_can_answer() {
        let request: RunQueryRequest =
            serde_json::from_value(json!({ "sql": "SELECT 1" })).expect("no project_name");
        assert_eq!(request.project_name, None);
    }
}
