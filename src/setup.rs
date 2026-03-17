use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
};

use crate::{
    config::{config_path, Config},
    error::{AppError, Result},
};

pub fn maybe_run_setup(config: &mut Config) -> Result<()> {
    if !config_path()?.exists() {
        run_setup_wizard(config)?;
    }
    Ok(())
}

pub fn force_run_setup(config: &mut Config) -> Result<()> {
    run_setup_wizard(config)
}

fn run_setup_wizard(config: &mut Config) -> Result<()> {
    println!("OpenClaw Node Widget - First-Time Setup");

    let node_script = detect_node_script();
    let detected = node_script
        .as_ref()
        .and_then(|path| parse_node_script(path).ok().flatten());

    config.gateway.url = resolve_gateway_url(detected.as_ref(), &config.gateway.url)?;
    config.gateway.token = resolve_gateway_token(detected.as_ref())?;

    match node_script {
        Some(path) => {
            #[cfg(windows)]
            {
                config.node.command = "cmd.exe".to_string();
                config.node.args = vec!["/c".to_string(), path.to_string_lossy().to_string()];
            }
            #[cfg(not(windows))]
            {
                config.node.command = path.to_string_lossy().to_string();
                config.node.args.clear();
            }
        }
        None => {
            let manual = prompt(
                "Enter Node command (e.g. openclaw node run or /path/to/node.sh):",
                Some(config.node.command.as_str()),
            )?;
            config.node.command = manual;
            config.node.args.clear();
        }
    }

    config.save()?;
    println!("Setup complete! Starting widget...");
    Ok(())
}

#[derive(Debug, Clone)]
struct ScriptDetection {
    host: Option<String>,
    port: Option<String>,
    token: Option<String>,
}

fn detect_node_script() -> Option<PathBuf> {
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

fn parse_node_script(path: &std::path::Path) -> Result<Option<ScriptDetection>> {
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

fn resolve_gateway_url(detected: Option<&ScriptDetection>, fallback: &str) -> Result<String> {
    if let Some(url) = detected_gateway_url(detected) {
        let use_detected = confirm(
            &format!("Detected Gateway URL: {url}. Use this? [Y/n]"),
            true,
        )?;
        if use_detected {
            return Ok(url);
        }
    }

    let default = if fallback.trim().is_empty() {
        None
    } else {
        Some(fallback)
    };
    prompt(
        "Enter Gateway URL (e.g. ws://192.168.1.100:18789):",
        default,
    )
}

fn resolve_gateway_token(detected: Option<&ScriptDetection>) -> Result<String> {
    if let Some(token) = detected.and_then(|d| d.token.clone()) {
        return Ok(token);
    }

    prompt("Enter Gateway Token (leave blank if none):", Some(""))
}

fn detected_gateway_url(detected: Option<&ScriptDetection>) -> Option<String> {
    let detection = detected?;
    let host = detection.host.as_ref()?;
    let port = detection.port.as_ref()?;
    Some(format!("ws://{host}:{port}"))
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

fn prompt(prompt: &str, default: Option<&str>) -> Result<String> {
    loop {
        if let Some(default) = default {
            if default.is_empty() {
                print!("{prompt} ");
            } else {
                print!("{prompt} [{default}] ");
            }
        } else {
            print!("{prompt} ");
        }
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let trimmed = input.trim();

        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }

        if let Some(default) = default {
            return Ok(default.to_string());
        }

        println!("A value is required.");
    }
}

fn confirm(prompt_text: &str, default_yes: bool) -> Result<bool> {
    print!("{prompt_text} ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let value = input.trim().to_lowercase();

    if value.is_empty() {
        return Ok(default_yes);
    }

    Ok(matches!(value.as_str(), "y" | "yes"))
}
