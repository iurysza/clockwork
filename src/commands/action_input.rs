//! Shared action-input validation for the managed job CLI.

use anyhow::{Result, bail};

use crate::model::action::{Action, HttpMethod};

/// Input limits from spec.
pub(crate) const MAX_COMMAND_LEN: usize = 32 * 1024;
pub(crate) const MAX_PROMPT_LEN: usize = 128 * 1024;
pub(crate) const MAX_TAGS: usize = 20;
pub(crate) const MAX_TAG_LEN: usize = 64;

/// Validate tag count and per-tag length.
pub(crate) fn validate_tags(tags: &[String]) -> Result<()> {
    if tags.len() > MAX_TAGS {
        bail!("Error: Maximum {MAX_TAGS} tags per job.");
    }
    for tag in tags {
        if tag.len() > MAX_TAG_LEN {
            bail!("Error: Tag '{tag}' exceeds maximum length of {MAX_TAG_LEN} characters.");
        }
    }
    Ok(())
}

/// Pure webhook construction for the managed-job planner. The service reads
/// the configuration once and passes the effective HTTP policy in its snapshot.
pub(crate) fn build_webhook_action_with_policy(
    url: &str,
    method: HttpMethod,
    headers: Vec<(String, String)>,
    body: Option<String>,
    allow_insecure_http: bool,
) -> Result<Action> {
    let parsed = url::Url::parse(url)
        .map_err(|e| anyhow::anyhow!("Error: Invalid webhook URL '{url}': {e}"))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        bail!("Error: Only http:// and https:// webhook URLs are supported.");
    }

    if parsed.scheme() == "http" && !allow_insecure_http {
        bail!(
            "Error: HTTP webhooks are blocked by default for security.\n\
             To allow: clockwork config allow_insecure_http true"
        );
    }

    Ok(Action::Webhook {
        url: url.to_string(),
        method,
        headers,
        body,
    })
}

/// Parse an optional HTTP method string, defaulting to POST.
pub(crate) fn parse_method(method: Option<&str>) -> Result<HttpMethod> {
    method
        .map(str::parse::<HttpMethod>)
        .transpose()
        .map_err(|e| anyhow::anyhow!("Error: {e}"))
        .map(Option::unwrap_or_default)
}

/// Parse repeatable `"Key: Value"` header lines (CLI form).
pub(crate) fn parse_header_lines(headers: &[String]) -> Result<Vec<(String, String)>> {
    let mut result = Vec::new();
    for h in headers {
        let (key, value) = h.split_once(':').ok_or_else(|| {
            anyhow::anyhow!("Error: Invalid header format '{h}'. Expected 'Key: Value'.")
        })?;
        result.push((key.trim().to_string(), value.trim().to_string()));
    }
    Ok(result)
}
