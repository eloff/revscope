use std::env;
use std::error::Error as StdError;
use std::fmt;

use serde_json::json;

const MAILCHANNELS_SEND_URL: &str = "https://api.mailchannels.net/tx/v1/send";
const DEFAULT_FROM_NAME: &str = "RevScope";

#[derive(Debug)]
pub(crate) enum EmailError {
    MissingEnv(&'static str),
    Request(String),
}

impl fmt::Display for EmailError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingEnv(name) => write!(f, "missing required environment variable {name}"),
            Self::Request(message) => write!(f, "{message}"),
        }
    }
}

impl StdError for EmailError {}

pub(crate) async fn send_password_reset_email(
    http: &reqwest::Client,
    to_email: &str,
    to_name: &str,
    reset_url: &str,
) -> Result<(), EmailError> {
    let plain_text = format!(
        "We received a request to reset your RevScope password.\n\nReset your password here:\n{reset_url}\n\nThis link expires in one hour. If you did not request this, you can ignore this email."
    );
    let html = format!(
        "<p>We received a request to reset your RevScope password.</p><p><a href=\"{}\">Reset your password</a></p><p>This link expires in one hour. If you did not request this, you can ignore this email.</p>",
        escape_html_attribute(reset_url),
    );
    send_email(
        http,
        to_email,
        to_name,
        "Reset your RevScope password",
        &plain_text,
        &html,
    )
    .await
}

pub(crate) async fn send_verification_email(
    http: &reqwest::Client,
    to_email: &str,
    to_name: &str,
    verify_url: &str,
) -> Result<(), EmailError> {
    let plain_text = format!(
        "Confirm this is your email address to finish setting up RevScope:\n{verify_url}\n\nUntil you do, Ask AI, Google Analytics imports and integrations stay off. This link expires in 7 days. If you didn't sign up for RevScope, you can ignore this email."
    );
    let html = format!(
        "<p>Confirm this is your email address to finish setting up RevScope.</p><p><a href=\"{}\">Confirm your email</a></p><p>Until you do, Ask AI, Google Analytics imports and integrations stay off. This link expires in 7 days. If you didn't sign up for RevScope, you can ignore this email.</p>",
        escape_html_attribute(verify_url),
    );
    send_email(
        http,
        to_email,
        to_name,
        "Confirm your RevScope email",
        &plain_text,
        &html,
    )
    .await
}

pub(crate) async fn send_invite_email(
    http: &reqwest::Client,
    to_email: &str,
    to_name: &str,
    inviter_name: &str,
    invite_url: &str,
) -> Result<(), EmailError> {
    let plain_text = format!(
        "{inviter_name} invited you to their RevScope analytics.\n\nChoose a password to accept:\n{invite_url}\n\nThis link expires in 7 days."
    );
    let html = format!(
        "<p>{} invited you to their RevScope analytics.</p><p><a href=\"{}\">Choose a password to accept</a></p><p>This link expires in 7 days.</p>",
        escape_html_attribute(inviter_name),
        escape_html_attribute(invite_url),
    );
    send_email(
        http,
        to_email,
        to_name,
        "You're invited to RevScope",
        &plain_text,
        &html,
    )
    .await
}

async fn send_email(
    http: &reqwest::Client,
    to_email: &str,
    to_name: &str,
    subject: &str,
    plain_text: &str,
    html: &str,
) -> Result<(), EmailError> {
    let api_key = required_env("MAILCHANNELS_API_KEY")?;
    let from_email = required_env("MAILCHANNELS_FROM_EMAIL")?;
    let from_name =
        optional_env("MAILCHANNELS_FROM_NAME").unwrap_or_else(|| DEFAULT_FROM_NAME.to_string());
    let send_url =
        optional_env("MAILCHANNELS_API_URL").unwrap_or_else(|| MAILCHANNELS_SEND_URL.to_string());

    let mut recipient = json!({ "email": to_email });
    if !to_name.trim().is_empty()
        && let Some(object) = recipient.as_object_mut()
    {
        object.insert("name".to_string(), json!(to_name.trim()));
    }

    let payload = json!({
        "personalizations": [
            { "to": [recipient] }
        ],
        "from": {
            "email": from_email,
            "name": from_name,
        },
        "subject": subject,
        "content": [
            {
                "type": "text/plain",
                "value": plain_text,
            },
            {
                "type": "text/html",
                "value": html,
            },
        ],
    });

    let response = http
        .post(send_url)
        .header("X-Api-Key", api_key)
        .json(&payload)
        .send()
        .await
        .map_err(|error| EmailError::Request(error.to_string()))?;

    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body = response.text().await.unwrap_or_default();
    Err(EmailError::Request(format!(
        "MailChannels returned {status}: {body}"
    )))
}

fn required_env(name: &'static str) -> Result<String, EmailError> {
    optional_env(name).ok_or(EmailError::MissingEnv(name))
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn escape_html_attribute(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            _ => escaped.push(character),
        }
    }
    escaped
}
