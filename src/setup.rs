use std::{fs, io, path::PathBuf};

use crate::error::{AppError, Result};

#[derive(Debug, Clone)]
pub struct ScriptDetection {
    pub host: Option<String>,
    pub port: Option<String>,
    pub token: Option<String>,
}

pub fn find_node_script() -> Option<PathBuf> {
    let home = dirs::home_dir()?;

    #[cfg(windows)]
    {
        let candidate = home.join(".openclaw").join("node.cmd");
        if candidate.exists() {
            return Some(candidate);
        }
    }

    #[cfg(not(windows))]
    {
        let candidate = home.join(".openclaw").join("node.sh");
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

pub fn parse_node_script(path: &std::path::Path) -> Result<Option<ScriptDetection>> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(AppError::Io(err)),
    };

    let host = find_flag_value(&content, "--host");
    let port = find_flag_value(&content, "--port");
    let token = find_gateway_token(&content);

    Ok(Some(ScriptDetection { host, port, token }))
}

fn find_flag_value(content: &str, flag: &str) -> Option<String> {
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.starts_with('#') || line.starts_with("REM") || line.starts_with("::") {
            continue;
        }

        let tokens: Vec<&str> = line.split_whitespace().collect();
        for (idx, token) in tokens.iter().enumerate() {
            if *token == flag {
                if let Some(next) = tokens.get(idx + 1) {
                    return Some(trim_token(next));
                }
            }
            if let Some(value) = token.strip_prefix(&format!("{flag}=")) {
                return Some(trim_token(value));
            }
        }
    }
    None
}

fn find_gateway_token(content: &str) -> Option<String> {
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if let Some(value) = line.strip_prefix("OPENCLAW_GATEWAY_TOKEN=") {
            return Some(trim_token(value));
        }
        if let Some(value) = line.strip_prefix("set OPENCLAW_GATEWAY_TOKEN=") {
            return Some(trim_token(value));
        }
        if let Some(value) = line.strip_prefix("export OPENCLAW_GATEWAY_TOKEN=") {
            return Some(trim_token(value));
        }
    }
    None
}

fn trim_token(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}
