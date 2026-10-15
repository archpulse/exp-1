# titler-rs

Rust-реализация для инференса модели [Cyronius/titler](https://huggingface.co/Cyronius/titler) на базе фреймворка [Candle](https://github.com/huggingface/candle).

Модель **titler** — это легковесный T5 (7.6M параметров, 3 слоя энкодера и 3 слоя декодера), обученный генерировать краткие заголовки (3–8 слов) по первому сообщению пользователя или фрагменту текста (SQL-запросы, логи, сниппеты).

## Особенности
- **100% Rust & Candle**: нативный инференс без Python, PyTorch или ONNX Runtime C++ библиотек.
- **Автоматическая загрузка с Hugging Face Hub**: кэширование весов в стандартный каталог `~/.cache/huggingface/hub/`.
- **Поддержка локального режима**: возможность загрузки из каталога с локальными файлами (`config.json`, `tokenizer.json`, `model.safetensors`).
- **Библиотека и CLI**: готовый крейт с публичным API (`titler::Titler`) и консольная утилита.
- **Поддержка стриминга / stdin / REPL**: передача аргументом, через pipe (`echo "..." | titler`) или в интерактивном режиме.

## Сборка

```bash
cargo build --release
```

## Использование CLI

### 1. Передача промпта аргументом:
```bash
./target/release/titler "how do I center a div horizontally with flexbox"
# Вывод: Centering a Div Humanly with Flexbox
```

### 2. Передача через STDIN (конвейер):
```bash
echo "SELECT c.name, SUM(o.total) FROM customers c GROUP BY c.name" | ./target/release/titler
# Вывод: SQL Query for Revenue Customer Orders
```

### 3. Интерактивный режим (REPL):
```bash
./target/release/titler
```

### 4. Параметры генерации:
```bash
./target/release/titler --help
```
Доступные флаги:
- `--max-tokens <N>`: максимальная длина генерации (по умолчанию: 32).
- `--temperature <F>`: температура сэмплинга (0.0 — жадный поиск argmax).
- `--top-p <F>`: nucleus sampling cutoff.
- `--no-repeat-ngram <N>`: запрет повторения n-грамм (по умолчанию: 2).
- `--repeat-penalty <F>`: штраф за повторение токенов.
- `--local-dir <DIR>`: путь к локальной папке с весами.

## Использование как Rust-библиотека

Добавьте в свой `Cargo.toml`:
```toml
[dependencies]
titler = { path = "../titler" } # или имя пакета
```

Пример кода:
```rust
use anyhow::Result;
use titler::{Titler, GenerateOptions};

fn main() -> Result<()> {
    // Загрузка с Hugging Face Hub (или Titler::from_local_dir("path/to/dir", None))
    let mut model = Titler::from_hub("Cyronius/titler", None)?;

    // Генерация с параметрами по умолчанию
    let title = model.generate("how do I center a div horizontally with flexbox")?;
    println!("Title: {title}");

    // Генерация с кастомными параметрами
    let options = GenerateOptions {
        max_new_tokens: 32,
        temperature: 0.0,
        no_repeat_ngram_size: 2,
        ..Default::default()
    };
    let title2 = model.generate_with_options("FATAL Database connection timed out", &options)?;
    println!("Title: {title2}");

    Ok(())
}
```
