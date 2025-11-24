// src/templates.rs
// Version: 3.1.0
// Created: 2024-11-17
// Updated: 2024-11-17
// 
// HTML template serving with JWT authentication context
// Serves HTML pages with proper headers and authentication state

use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use std::collections::HashMap;

/// Template engine for serving HTML pages with dynamic content injection
pub struct TemplateEngine {
    base_path: String,
}

impl TemplateEngine {
    /// Create a new template engine instance with base path
    pub fn new(base_path: &str) -> Self {
        Self {
            base_path: base_path.to_string(),
        }
    }

    /// Render an HTML template file with optional variables
    pub fn render(
        &self,
        template_name: &str,
        vars: Option<&HashMap<String, String>>,
    ) -> Result<String, String> {
        let path = format!("{}/{}.html", self.base_path, template_name);
        
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read template {}: {}", path, e))?;
        
        // If variables are provided, do simple string replacement
        let mut output = content;
        if let Some(variables) = vars {
            for (key, value) in variables {
                let placeholder = format!("{{{{{}}}}}", key);
                output = output.replace(&placeholder, value);
            }
        }
        
        Ok(output)
    }
}

impl Default for TemplateEngine {
    fn default() -> Self {
        Self::new("static")
    }
}

/// Serve the events monitoring page
pub async fn serve_events() -> Response {
    serve_static_html("events.html").await
}

/// Serve the tools page (SCTE-35 toolkit)
pub async fn serve_tools() -> Response {
    serve_static_html("tools.html").await
}

/// Serve the docs/API documentation page
pub async fn serve_docs() -> Response {
    serve_static_html("docs.html").await
}

/// Serve the users management page
pub async fn serve_users() -> Response {
    serve_static_html("users.html").await
}

/// Serve the API tokens management page
pub async fn serve_tokens() -> Response {
    serve_static_html("tokens.html").await
}

/// Serve the login page
pub async fn serve_login() -> Response {
    serve_static_html("login.html").await
}

/// Helper function to serve static HTML files
async fn serve_static_html(filename: &str) -> Response {
    let path = format!("static/{}", filename);
    
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => Html(content).into_response(),
        Err(e) => {
            tracing::error!("Failed to read {}: {}", path, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to load {}", filename),
            )
                .into_response()
        }
    }
}