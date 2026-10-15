use std::path::Path;
use anyhow::{anyhow, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::t5::{Config, T5ForConditionalGeneration};
use hf_hub::api::sync::Api;
use tokenizers::Tokenizer;

pub mod lang;
pub mod translate;

pub use lang::{code_to_language_name, detect_language, DetectionResult};
pub use translate::{translate_to_english, TranslationResult};

/// Result of multilingual title generation, containing original prompt, detected language, and optional translated prompt.
#[derive(Debug, Clone)]
pub struct MultilingualTitleResult {
    pub title: String,
    pub original_prompt: String,
    pub detected_lang: String,
    pub translated_prompt: Option<String>,
    pub was_translated: bool,
}

/// Generation options for Titler.
#[derive(Debug, Clone)]
pub struct GenerateOptions {
    /// Maximum number of tokens to generate.
    pub max_new_tokens: usize,
    /// Sampling temperature (0.0 for greedy decoding).
    pub temperature: f64,
    /// Top-p (nucleus sampling) cutoff if temperature > 0.
    pub top_p: Option<f64>,
    /// Penalty applied to repeating previously generated tokens.
    pub repeat_penalty: f32,
    /// Size of n-grams forbidden from repeating (default: 2).
    pub no_repeat_ngram_size: usize,
}

impl Default for GenerateOptions {
    fn default() -> Self {
        Self {
            max_new_tokens: 32,
            temperature: 0.0,
            top_p: None,
            repeat_penalty: 1.0,
            no_repeat_ngram_size: 2,
        }
    }
}

/// Titler model runner for generating concise titles from text prompts.
pub struct Titler {
    model: T5ForConditionalGeneration,
    tokenizer: Tokenizer,
    config: Config,
    device: Device,
}

impl Titler {
    /// Loads the model from Hugging Face Hub (e.g. "Cyronius/titler").
    pub fn from_hub(model_id: &str, device: Option<Device>) -> Result<Self> {
        let device = device.unwrap_or(Device::Cpu);
        let api = Api::new()?;
        let repo = api.model(model_id.to_string());

        let config_path = repo.get("config.json")?;
        let tokenizer_path = repo.get("tokenizer.json")?;
        let weights_path = repo.get("model.safetensors")?;

        Self::from_files(&config_path, &tokenizer_path, &weights_path, device)
    }

    /// Loads the model from a local directory containing config.json, tokenizer.json, and model.safetensors.
    pub fn from_local_dir<P: AsRef<Path>>(dir: P, device: Option<Device>) -> Result<Self> {
        let dir = dir.as_ref();
        let config_path = dir.join("config.json");
        let tokenizer_path = dir.join("tokenizer.json");
        let weights_path = dir.join("model.safetensors");

        if !config_path.exists() {
            return Err(anyhow!("Missing config.json in {}", dir.display()));
        }
        if !tokenizer_path.exists() {
            return Err(anyhow!("Missing tokenizer.json in {}", dir.display()));
        }
        if !weights_path.exists() {
            return Err(anyhow!("Missing model.safetensors in {}", dir.display()));
        }

        Self::from_files(&config_path, &tokenizer_path, &weights_path, device.unwrap_or(Device::Cpu))
    }

    /// Loads the model from explicit file paths.
    pub fn from_files<P: AsRef<Path>>(
        config_path: P,
        tokenizer_path: P,
        weights_path: P,
        device: Device,
    ) -> Result<Self> {
        let config_str = std::fs::read_to_string(config_path.as_ref())?;
        let config: Config = serde_json::from_str(&config_str)?;

        let tokenizer = Tokenizer::from_file(tokenizer_path.as_ref())
            .map_err(|e| anyhow!("Failed to load tokenizer: {e}"))?;

        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path.as_ref().to_path_buf()], DType::F32, &device)?
        };

        let model = T5ForConditionalGeneration::load(vb, &config)?;

        Ok(Self {
            model,
            tokenizer,
            config,
            device,
        })
    }

    /// Generates a title with default generation parameters (detects language and translates if not English).
    pub fn generate(&mut self, prompt: &str) -> Result<String> {
        self.generate_with_options(prompt, &GenerateOptions::default())
    }

    /// Generates a title with custom generation options (detects language and translates if not English).
    pub fn generate_with_options(&mut self, prompt: &str, options: &GenerateOptions) -> Result<String> {
        let res = self.generate_multilingual(prompt, options)?;
        Ok(res.title)
    }

    /// Generates a title directly without translation (raw English input to T5).
    pub fn generate_raw(&mut self, prompt: &str, options: &GenerateOptions) -> Result<String> {
        self.generate_internal(prompt, options)
    }

    /// Full multilingual pipeline returning detailed metadata:
    /// detects language, translates to English if needed, and generates an English title.
    pub fn generate_multilingual(
        &mut self,
        prompt: &str,
        options: &GenerateOptions,
    ) -> Result<MultilingualTitleResult> {
        let detection = detect_language(prompt);
        if detection.is_english {
            let title = self.generate_internal(prompt, options)?;
            Ok(MultilingualTitleResult {
                title,
                original_prompt: prompt.to_string(),
                detected_lang: detection.lang_name,
                translated_prompt: None,
                was_translated: false,
            })
        } else {
            let translation = translate_to_english(prompt)?;
            let detected_lang = if let Some(ref src) = translation.detected_source {
                code_to_language_name(src).to_string()
            } else {
                detection.lang_name
            };

            let title = self.generate_internal(&translation.translated_text, options)?;
            Ok(MultilingualTitleResult {
                title,
                original_prompt: prompt.to_string(),
                detected_lang,
                translated_prompt: Some(translation.translated_text),
                was_translated: true,
            })
        }
    }

    /// Internal T5 generation routine.
    fn generate_internal(&mut self, prompt: &str, options: &GenerateOptions) -> Result<String> {
        let mut input_tokens = self
            .tokenizer
            .encode(prompt, true)
            .map_err(|e| anyhow!("Tokenization error: {e}"))?
            .get_ids()
            .to_vec();

        // T5 models require an EOS token at the end of input sequence
        let eos_id = self.config.eos_token_id as u32;
        if input_tokens.last() != Some(&eos_id) {
            input_tokens.push(eos_id);
        }

        let input_tensor = Tensor::new(&input_tokens[..], &self.device)?.unsqueeze(0)?;

        self.model.clear_kv_cache();
        let encoder_output = self.model.encode(&input_tensor)?;

        let start_token = self.config.decoder_start_token_id.unwrap_or(0) as u32;
        let mut generated_tokens: Vec<u32> = Vec::new();
        let mut current_token = start_token;

        for _step in 0..options.max_new_tokens {
            let decoder_input = if self.config.use_cache {
                Tensor::new(&[current_token], &self.device)?.unsqueeze(0)?
            } else {
                let mut seq = vec![start_token];
                seq.extend_from_slice(&generated_tokens);
                Tensor::new(&seq[..], &self.device)?.unsqueeze(0)?
            };

            let logits = self.model.decode(&decoder_input, &encoder_output)?;
            let mut logits_vec: Vec<f32> = logits.squeeze(0)?.to_vec1()?;

            // Apply repetition penalty
            if (options.repeat_penalty - 1.0).abs() > 1e-4 {
                for &prev in &generated_tokens {
                    let idx = prev as usize;
                    if idx < logits_vec.len() {
                        if logits_vec[idx] > 0.0 {
                            logits_vec[idx] /= options.repeat_penalty;
                        } else {
                            logits_vec[idx] *= options.repeat_penalty;
                        }
                    }
                }
            }

            // Apply no-repeat n-gram size ban
            if options.no_repeat_ngram_size > 0 && generated_tokens.len() >= options.no_repeat_ngram_size {
                let n = options.no_repeat_ngram_size;
                let prefix = &generated_tokens[generated_tokens.len() - (n - 1)..];
                for i in 0..=(generated_tokens.len() - n) {
                    if &generated_tokens[i..i + n - 1] == prefix {
                        let banned_token = generated_tokens[i + n - 1] as usize;
                        if banned_token < logits_vec.len() {
                            logits_vec[banned_token] = f32::NEG_INFINITY;
                        }
                    }
                }
            }

            let next_token = if options.temperature <= 0.0 {
                // Greedy selection
                let mut best_idx = 0u32;
                let mut best_val = f32::NEG_INFINITY;
                for (idx, &val) in logits_vec.iter().enumerate() {
                    if val > best_val {
                        best_val = val;
                        best_idx = idx as u32;
                    }
                }
                best_idx
            } else {
                // Temperature and top-p sampling
                sample_token(&logits_vec, options.temperature, options.top_p)?
            };

            if next_token == eos_id {
                break;
            }

            generated_tokens.push(next_token);
            current_token = next_token;
        }

        let raw_title = self
            .tokenizer
            .decode(&generated_tokens, true)
            .map_err(|e| anyhow!("Decode error: {e}"))?;

        let title = clean_title(raw_title.trim());

        Ok(title)
    }

    /// Access the underlying tokenizer.
    pub fn tokenizer(&self) -> &Tokenizer {
        &self.tokenizer
    }

    /// Access the model config.
    pub fn config(&self) -> &Config {
        &self.config
    }
}

/// Helper function to sample a token from logits using temperature and optional top-p.
fn sample_token(logits: &[f32], temperature: f64, top_p: Option<f64>) -> Result<u32> {
    use rand::Rng;

    let scaled: Vec<f32> = logits.iter().map(|&x| (x as f64 / temperature) as f32).collect();
    let max_val = scaled.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f32> = scaled.iter().map(|&x| (x - max_val).exp()).collect();
    let sum_exp: f32 = exps.iter().sum();
    let mut probs: Vec<(usize, f32)> = exps.into_iter().enumerate().map(|(i, e)| (i, e / sum_exp)).collect();

    // Sort descending by probability
    probs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Top-p truncation
    let filtered_probs = if let Some(p) = top_p {
        let mut cumulative = 0.0f32;
        let mut cutoff_idx = probs.len();
        for (idx, (_, prob)) in probs.iter().enumerate() {
            cumulative += *prob;
            if cumulative >= p as f32 {
                cutoff_idx = idx + 1;
                break;
            }
        }
        &probs[..cutoff_idx]
    } else {
        &probs[..]
    };

    let total: f32 = filtered_probs.iter().map(|(_, p)| p).sum();
    let mut rng = rand::rng();
    let roll: f32 = rng.random_range(0.0..total);

    let mut accum = 0.0f32;
    for &(token_idx, prob) in filtered_probs {
        accum += prob;
        if roll <= accum {
            return Ok(token_idx as u32);
        }
    }

    Ok(filtered_probs.last().map(|&(idx, _)| idx as u32).unwrap_or(0))
}

/// Normalizes common awkward contractions and spacing in generated titles.
fn clean_title(title: &str) -> String {
    let mut s = title.trim().to_string();
    s = s.replace("Why Is Not ", "Why Isn't ");
    s = s.replace("Is Not ", "Isn't ");
    s = s.replace("Does Not ", "Doesn't ");
    s = s.replace("Do Not ", "Don't ");
    s = s.replace("Can Not ", "Cannot ");
    s
}
