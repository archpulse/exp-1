use whatlang::{detect, Lang};

#[derive(Debug, Clone)]
pub struct DetectionResult {
    pub lang_code: String,
    pub lang_name: String,
    pub is_english: bool,
    pub confidence: f64,
    pub is_reliable: bool,
}

/// Converts an ISO 639-1 / 639-3 language code to a human-readable English name.
pub fn code_to_language_name(code: &str) -> &'static str {
    match code.to_lowercase().as_str() {
        "en" | "eng" => "English",
        "ru" | "rus" => "Russian",
        "uk" | "ukr" => "Ukrainian",
        "be" | "bel" => "Belarusian",
        "de" | "deu" => "German",
        "fr" | "fra" => "French",
        "es" | "spa" => "Spanish",
        "it" | "ita" => "Italian",
        "pt" | "por" => "Portuguese",
        "zh" | "zho" | "cmn" | "zh-cn" | "zh-tw" => "Chinese",
        "ja" | "jpn" => "Japanese",
        "ko" | "kor" => "Korean",
        "vi" | "vie" => "Vietnamese",
        "th" | "tha" => "Thai",
        "pl" | "pol" => "Polish",
        "cs" | "ces" => "Czech",
        "tr" | "tur" => "Turkish",
        "ar" | "ara" => "Arabic",
        "he" | "heb" => "Hebrew",
        "hi" | "hin" => "Hindi",
        "id" | "ind" => "Indonesian",
        "nl" | "nld" => "Dutch",
        "sv" | "swe" => "Swedish",
        "fi" | "fin" => "Finnish",
        "da" | "dan" => "Danish",
        "no" | "nob" => "Norwegian",
        "el" | "ell" => "Greek",
        "ro" | "ron" => "Romanian",
        "bg" | "bul" => "Bulgarian",
        _ => "Non-English",
    }
}

/// Enhanced language detection with Cyrillic alphabet disambiguation.
pub fn detect_language(text: &str) -> DetectionResult {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return DetectionResult {
            lang_code: "eng".to_string(),
            lang_name: "English".to_string(),
            is_english: true,
            confidence: 1.0,
            is_reliable: true,
        };
    }

    let lower = trimmed.to_lowercase();

    // Specific Cyrillic alphabet disambiguation (rules that trigrams often get wrong on short phrases)
    let has_ru_unique = lower.chars().any(|c| c == 'ы' || c == 'э' || c == 'ъ' || c == 'ё');
    let has_uk_unique = lower.chars().any(|c| c == 'і' || c == 'ї' || c == 'є' || c == 'ґ');
    let has_be_unique = lower.chars().any(|c| c == 'ў');

    // Common Russian function words
    let has_ru_words = lower.split_whitespace().any(|w| {
        matches!(
            w.trim_matches(|c: char| !c.is_alphabetic()),
            "почему" | "зачем" | "как" | "что" | "это" | "духовка" | "работает" | "когда" | "где" | "куда" | "откуда"
        )
    });

    if has_ru_unique || (has_ru_words && !has_uk_unique) {
        return DetectionResult {
            lang_code: "ru".to_string(),
            lang_name: "Russian".to_string(),
            is_english: false,
            confidence: 0.99,
            is_reliable: true,
        };
    }

    if has_uk_unique {
        return DetectionResult {
            lang_code: "uk".to_string(),
            lang_name: "Ukrainian".to_string(),
            is_english: false,
            confidence: 0.99,
            is_reliable: true,
        };
    }

    if has_be_unique {
        return DetectionResult {
            lang_code: "be".to_string(),
            lang_name: "Belarusian".to_string(),
            is_english: false,
            confidence: 0.99,
            is_reliable: true,
        };
    }

    // Fall back to whatlang for other languages
    if let Some(info) = detect(trimmed) {
        let is_english = info.lang() == Lang::Eng;
        DetectionResult {
            lang_code: info.lang().code().to_string(),
            lang_name: info.lang().eng_name().to_string(),
            is_english,
            confidence: info.confidence(),
            is_reliable: info.is_reliable(),
        }
    } else {
        let has_non_ascii_letters = trimmed.chars().any(|c| c.is_alphabetic() && !c.is_ascii());
        if has_non_ascii_letters {
            DetectionResult {
                lang_code: "und".to_string(),
                lang_name: "Non-English".to_string(),
                is_english: false,
                confidence: 0.0,
                is_reliable: false,
            }
        } else {
            DetectionResult {
                lang_code: "eng".to_string(),
                lang_name: "English".to_string(),
                is_english: true,
                confidence: 0.5,
                is_reliable: false,
            }
        }
    }
}
