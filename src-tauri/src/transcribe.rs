use anyhow::{Context, Result};
use reqwest::multipart;

pub async fn transcribe_audio(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    wav_data: Vec<u8>,
    language: Option<&str>,
) -> Result<String> {
    let file_part = multipart::Part::bytes(wav_data)
        .file_name("audio.wav")
        .mime_str("audio/wav")?;

    let mut form = multipart::Form::new()
        .part("file", file_part)
        .text("model", model.to_string());

    if let Some(lang) = language {
        if lang != "auto" {
            form = form.text("language", lang.to_string());
        }
    }

    let resp = client
        .post("https://api.openai.com/v1/audio/transcriptions")
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await
        .context("Failed to send transcription request")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("API error {}: {}", status, body);
    }

    let json: serde_json::Value = resp.json().await.context("Failed to parse API response")?;
    let text = json["text"]
        .as_str()
        .context("Missing 'text' field in response")?
        .to_string();

    Ok(text)
}
