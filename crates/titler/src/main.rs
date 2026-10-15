use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::PathBuf;
use anyhow::Result;
use clap::Parser;
use titler::{GenerateOptions, Titler};

#[derive(Parser, Debug)]
#[command(name = "titler", about = "Generate concise titles from text prompts using Cyronius/titler")]
struct Cli {
    /// Text prompt to generate a title for (if omitted, reads from stdin or interactive REPL)
    #[arg(value_name = "PROMPT")]
    prompt: Option<String>,

    /// Explicit prompt flag (alternative to positional argument)
    #[arg(short = 'p', long = "prompt")]
    prompt_flag: Option<String>,

    /// Hugging Face Hub model ID
    #[arg(long, default_value = "Cyronius/titler")]
    model_id: String,

    /// Load model from a local directory instead of Hugging Face Hub
    #[arg(long, value_name = "DIR")]
    local_dir: Option<PathBuf>,

    /// Maximum tokens to generate
    #[arg(long, default_value_t = 32)]
    max_tokens: usize,

    /// Sampling temperature (0.0 for greedy decoding)
    #[arg(long, default_value_t = 0.0)]
    temperature: f64,

    /// Nucleus sampling cutoff probability
    #[arg(long)]
    top_p: Option<f64>,

    /// Size of n-grams forbidden from repeating
    #[arg(long, default_value_t = 2)]
    no_repeat_ngram: usize,

    /// Penalty for repeating tokens (1.0 = none)
    #[arg(long, default_value_t = 1.0)]
    repeat_penalty: f32,

    /// Show language detection and translation details
    #[arg(short = 'v', long)]
    verbose: bool,

    /// Disable automatic translation (send raw text directly to model)
    #[arg(long)]
    no_translate: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let options = GenerateOptions {
        max_new_tokens: cli.max_tokens,
        temperature: cli.temperature,
        top_p: cli.top_p,
        repeat_penalty: cli.repeat_penalty,
        no_repeat_ngram_size: cli.no_repeat_ngram,
    };

    let mut titler = if let Some(local_path) = &cli.local_dir {
        Titler::from_local_dir(local_path, None)?
    } else {
        Titler::from_hub(&cli.model_id, None)?
    };

    // Determine the prompt source
    let input_prompt = cli.prompt_flag.or(cli.prompt);

    if let Some(prompt) = input_prompt {
        if cli.no_translate {
            let title = titler.generate_raw(&prompt, &options)?;
            println!("{title}");
        } else if cli.verbose {
            let res = titler.generate_multilingual(&prompt, &options)?;
            if res.was_translated {
                if let Some(ref trans) = res.translated_prompt {
                    eprintln!("[Language: {} | Translated: \"{}\"]", res.detected_lang, trans);
                }
            } else {
                eprintln!("[Language: {}]", res.detected_lang);
            }
            println!("{}", res.title);
        } else {
            let title = titler.generate_with_options(&prompt, &options)?;
            println!("{title}");
        }
        return Ok(());
    }

    // Check if stdin is piped
    let stdin = io::stdin();
    if !stdin.is_terminal() {
        let mut buffer = String::new();
        stdin.lock().read_to_string(&mut buffer)?;
        let trimmed = buffer.trim();
        if !trimmed.is_empty() {
            if cli.no_translate {
                let title = titler.generate_raw(trimmed, &options)?;
                println!("{title}");
            } else if cli.verbose {
                let res = titler.generate_multilingual(trimmed, &options)?;
                if res.was_translated {
                    if let Some(ref trans) = res.translated_prompt {
                        eprintln!("[Language: {} | Translated: \"{}\"]", res.detected_lang, trans);
                    }
                } else {
                    eprintln!("[Language: {}]", res.detected_lang);
                }
                println!("{}", res.title);
            } else {
                let title = titler.generate_with_options(trimmed, &options)?;
                println!("{title}");
            }
        }
        return Ok(());
    }

    // Interactive REPL mode
    println!("titler: interactive mode (type 'exit' or Ctrl+D to quit)");
    let mut stdout = io::stdout();
    let reader = io::BufReader::new(io::stdin());

    for line_result in reader.lines() {
        let line = line_result?;
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("exit") || trimmed.eq_ignore_ascii_case("quit") {
            break;
        }
        if trimmed.is_empty() {
            continue;
        }

        if cli.no_translate {
            let title = titler.generate_raw(trimmed, &options)?;
            println!("> {title}");
        } else {
            let res = titler.generate_multilingual(trimmed, &options)?;
            if res.was_translated {
                if let Some(ref trans) = res.translated_prompt {
                    println!("[Language: {} -> \"{}\"]", res.detected_lang, trans);
                }
            } else {
                println!("[Language: {}]", res.detected_lang);
            }
            println!("> {}", res.title);
        }
        stdout.flush()?;
    }

    Ok(())
}


