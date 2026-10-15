use std::time::Duration;
use anyhow::{anyhow, Result};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct TranslationResult {
    pub translated_text: String,
    pub detected_source: Option<String>,
}

/// Translates text to English using Google Chrome extension endpoint (clients5),
/// falling back to Google gtx and MyMemory if needed.
pub fn translate_to_english(text: &str) -> Result<TranslationResult> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(TranslationResult {
            translated_text: String::new(),
            detected_source: None,
        });
    }

    // Attempt primary engine: Google clients5 (Chrome translation API, highly reliable & accurate)
    if let Ok(res) = translate_via_clients5(trimmed) {
        if !res.translated_text.trim().is_empty() {
            return Ok(res);
        }
    }

    // Attempt secondary engine: Google gtx
    if let Ok(res) = translate_via_google_gtx(trimmed) {
        if !res.translated_text.trim().is_empty() {
            return Ok(res);
        }
    }

    // Attempt tertiary fallback engine: MyMemory
    translate_via_mymemory(trimmed)
}

fn translate_via_clients5(text: &str) -> Result<TranslationResult> {
    let encoded = urlencoding::encode(text);
    let url = format!(
        "https://clients5.google.com/translate_a/t?client=dict-chrome-ex&sl=auto&tl=en&q={encoded}"
    );

    let response = ureq::get(&url)
        .set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .timeout(Duration::from_secs(10))
        .call()
        .map_err(|e| anyhow!("Clients5 HTTP request failed: {e}"))?
        .into_string()
        .map_err(|e| anyhow!("Failed to read Clients5 body: {e}"))?;

    let json: Value = serde_json::from_str(&response)
        .map_err(|e| anyhow!("Failed to parse Clients5 JSON: {e}"))?;

    let mut translated = String::new();
    let mut detected_source = None;

    if let Some(arr) = json.as_array() {
        for item in arr {
            if let Some(seg) = item.as_array() {
                if let Some(chunk) = seg.get(0).and_then(|v| v.as_str()) {
                    translated.push_str(chunk);
                }
                if detected_source.is_none() {
                    if let Some(lang) = seg.get(1).and_then(|v| v.as_str()) {
                        detected_source = Some(lang.to_string());
                    }
                }
            } else if let Some(str_chunk) = item.as_str() {
                translated.push_str(str_chunk);
            }
        }
    }

    if translated.trim().is_empty() {
        return Err(anyhow!("Empty translation response from Clients5"));
    }

    Ok(TranslationResult {
        translated_text: translated.trim().to_string(),
        detected_source,
    })
}

fn translate_via_google_gtx(text: &str) -> Result<TranslationResult> {
    let encoded = urlencoding::encode(text);
    let url = format!(
        "https://translate.googleapis.com/translate_a/single?client=gtx&sl=auto&tl=en&dt=t&q={encoded}"
    );

    let response = ureq::get(&url)
        .set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        .timeout(Duration::from_secs(10))
        .call()
        .map_err(|e| anyhow!("Google gtx HTTP request failed: {e}"))?
        .into_string()
        .map_err(|e| anyhow!("Failed to read response body: {e}"))?;

    let json: Value = serde_json::from_str(&response)
        .map_err(|e| anyhow!("Failed to parse Google gtx JSON: {e}"))?;

    let segments = json
        .get(0)
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("Unexpected JSON format: missing segments array"))?;

    let mut translated = String::new();
    for seg in segments {
        if let Some(part) = seg.get(0).and_then(|p| p.as_str()) {
            translated.push_str(part);
        }
    }

    let detected_source = json
        .get(2)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(TranslationResult {
        translated_text: translated.trim().to_string(),
        detected_source,
    })
}

fn translate_via_mymemory(text: &str) -> Result<TranslationResult> {
    let encoded = urlencoding::encode(text);
    let url = format!(
        "https://api.mymemory.translated.net/get?q={encoded}&langpair=autodetect|en"
    );

    let response = ureq::get(&url)
        .set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        .timeout(Duration::from_secs(10))
        .call()
        .map_err(|e| anyhow!("MyMemory HTTP request failed: {e}"))?
        .into_string()
        .map_err(|e| anyhow!("Failed to read MyMemory body: {e}"))?;

    let json: Value = serde_json::from_str(&response)
        .map_err(|e| anyhow!("Failed to parse MyMemory JSON: {e}"))?;

    let translated = json
        .pointer("/responseData/translatedText")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing translatedText in MyMemory response"))?;

    let detected_source = json
        .pointer("/responseData/detectedLanguage")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(TranslationResult {
        translated_text: translated.trim().to_string(),
        detected_source,
    })
}
