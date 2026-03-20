use crate::config::VoiceConfig;

/// Transcribe base64-encoded WAV audio to text.
///
/// Tries local `whisper` CLI first; falls back to OpenAI Whisper API
/// if an API key is configured.
pub async fn transcribe(audio_base64: &str, config: &VoiceConfig) -> Result<String, String> {
    if audio_base64.is_empty() {
        return Err("empty audio data".to_string());
    }

    let audio_bytes = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        audio_base64,
    )
    .map_err(|e| format!("invalid base64: {e}"))?;

    if audio_bytes.is_empty() {
        return Err("empty audio data after decode".to_string());
    }

    // Write to temp file
    let tmp_dir = std::env::temp_dir();
    let tmp_path = tmp_dir.join("openclaw_voice_input.wav");
    tokio::fs::write(&tmp_path, &audio_bytes)
        .await
        .map_err(|e| format!("failed to write temp file: {e}"))?;

    let language_args = if config.language != "auto" {
        vec!["--language".to_string(), config.language.clone()]
    } else {
        vec![]
    };

    // Try local whisper CLI first (unless provider is explicitly openai)
    if config.provider != "openai" {
        match try_local_whisper(&tmp_path, &language_args).await {
            Ok(text) => {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Ok(text);
            }
            Err(e) => {
                tracing::debug!("local whisper not available: {e}");
                // Fall through to OpenAI API if key configured
            }
        }
    }

    // Try OpenAI Whisper API
    if let Some(api_key) = &config.openai_api_key {
        let result = try_openai_whisper(&tmp_path, api_key, &config.language).await;
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return result;
    }

    let _ = tokio::fs::remove_file(&tmp_path).await;
    Err("no whisper provider available (install whisper CLI or configure openai_api_key)".to_string())
}

async fn try_local_whisper(
    wav_path: &std::path::Path,
    language_args: &[String],
) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new("whisper");
    cmd.arg(wav_path.to_str().unwrap_or(""))
        .arg("--model")
        .arg("base")
        .arg("--output_format")
        .arg("txt")
        .arg("--output_dir")
        .arg(wav_path.parent().unwrap_or(std::path::Path::new("/tmp")));

    for arg in language_args {
        cmd.arg(arg);
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("whisper command failed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("whisper failed: {stderr}"));
    }

    // Read the output .txt file
    let txt_path = wav_path.with_extension("txt");
    let text = tokio::fs::read_to_string(&txt_path)
        .await
        .map_err(|e| format!("failed to read whisper output: {e}"))?;
    let _ = tokio::fs::remove_file(&txt_path).await;

    Ok(text.trim().to_string())
}

async fn try_openai_whisper(
    wav_path: &std::path::Path,
    api_key: &str,
    language: &str,
) -> Result<String, String> {
    let file_bytes = tokio::fs::read(wav_path)
        .await
        .map_err(|e| format!("failed to read audio file: {e}"))?;

    let file_part = reqwest::multipart::Part::bytes(file_bytes)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| format!("mime error: {e}"))?;

    let mut form = reqwest::multipart::Form::new()
        .text("model", "whisper-1")
        .part("file", file_part);

    if language != "auto" {
        form = form.text("language", language.to_string());
    }

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.openai.com/v1/audio/transcriptions")
        .header("Authorization", format!("Bearer {api_key}"))
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("OpenAI API request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("OpenAI API error {status}: {body}"));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("failed to parse OpenAI response: {e}"))?;

    body.get("text")
        .and_then(|t| t.as_str())
        .map(|s| s.trim().to_string())
        .ok_or_else(|| "no text in OpenAI response".to_string())
}

/// Validate base64 audio data without transcribing.
#[allow(dead_code)]
pub fn validate_audio_data(audio_base64: &str) -> Result<usize, String> {
    if audio_base64.is_empty() {
        return Err("empty audio data".to_string());
    }
    let bytes = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        audio_base64,
    )
    .map_err(|e| format!("invalid base64: {e}"))?;
    if bytes.is_empty() {
        return Err("empty audio data after decode".to_string());
    }
    // Max 30 seconds of WAV at 48kHz stereo 16-bit ≈ ~5.5MB
    let max_size = 6 * 1024 * 1024;
    if bytes.len() > max_size {
        return Err(format!(
            "audio too large: {} bytes (max {})",
            bytes.len(),
            max_size
        ));
    }
    Ok(bytes.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_empty_audio_returns_error() {
        assert!(validate_audio_data("").is_err());
    }

    #[test]
    fn validate_invalid_base64_returns_error() {
        assert!(validate_audio_data("not-valid-base64!!!").is_err());
    }

    #[test]
    fn validate_valid_base64_returns_size() {
        use base64::Engine;
        let data = vec![0u8; 100];
        let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
        let result = validate_audio_data(&encoded);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 100);
    }

    #[test]
    fn validate_too_large_audio_returns_error() {
        use base64::Engine;
        let data = vec![0u8; 7 * 1024 * 1024]; // 7MB
        let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
        let result = validate_audio_data(&encoded);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too large"));
    }

    #[tokio::test]
    async fn transcribe_empty_audio_returns_error() {
        let config = VoiceConfig::default();
        let result = transcribe("", &config).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty"));
    }

    #[tokio::test]
    async fn transcribe_invalid_base64_returns_error() {
        let config = VoiceConfig::default();
        let result = transcribe("!!!invalid!!!", &config).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("base64"));
    }
}
