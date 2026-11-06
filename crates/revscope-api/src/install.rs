//! The tracker's install snippet: the queue stub that loads the script.

use serde::{Deserialize, Serialize};

const PROJECT_PLACEHOLDER: &str = "REPLACE_WITH_PROJECT_NAME";

/// The stub `tracker/INSTALL.md` documents; a test here keeps the two identical.
const STUB: &str = r#"<script>
  (function (w, n, u, c) {
    if (u && typeof u === "object") {
      c = u;
      u = null;
    }
    c = c || {};
    u = u || "https://in.revscope.co/";
    c.url = u;
    u += n + ".js";
    c.project = n;
    n = c.name || "dispatch";
    if (typeof w[n] === "function") return;
    var q = [];
    function f() {
      q.push([].slice.call(arguments));
    }
    f.q = f.a = q;
    f.c = c;
    w[n] = f;
    var s = document.createElement("script");
    s.src = u;
    s.defer = true;
    document.head.appendChild(s);
  })(window, "REPLACE_WITH_PROJECT_NAME");
</script>"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallResponse {
    pub project: String,
    /// Where the tracker reports, when that isn't the stub's default of
    /// `https://in.revscope.co/`: a custom domain or a self-hosted server.
    pub server_url: Option<String>,
    pub snippet: String,
}

/// The snippet for `project`, reporting to `server_url` (ending in a slash), or
/// to the stub's default when there is none.
pub fn render_snippet(project: &str, server_url: Option<&str>) -> String {
    let arguments = match server_url {
        Some(url) => format!("\"{project}\", \"{url}\""),
        None => format!("\"{project}\""),
    };
    STUB.replace(&format!("\"{PROJECT_PLACEHOLDER}\""), &arguments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_stub_is_the_one_the_install_guide_documents() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tracker/INSTALL.md");
        let guide = std::fs::read_to_string(path).expect("tracker/INSTALL.md");
        assert!(guide.contains(STUB));
    }

    #[test]
    fn the_project_is_filled_in_and_the_default_server_is_kept() {
        let snippet = render_snippet("shop_example", None);
        assert!(snippet.contains(r#"})(window, "shop_example");"#));
        assert!(!snippet.contains(PROJECT_PLACEHOLDER));
    }

    #[test]
    fn a_server_url_becomes_the_third_argument() {
        let snippet = render_snippet("shop_example", Some("https://stats.example.com/"));
        assert!(snippet.contains(r#"})(window, "shop_example", "https://stats.example.com/");"#));
    }
}
