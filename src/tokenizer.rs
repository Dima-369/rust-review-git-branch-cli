use std::sync::OnceLock;
use tiktoken_rs::{CoreBPE, cl100k_base};

static TOKENIZER: OnceLock<CoreBPE> = OnceLock::new();

fn get_tokenizer() -> &'static CoreBPE {
    TOKENIZER.get_or_init(|| cl100k_base().expect("Failed to load cl100k_base tokenizer"))
}

/// Get token count for text using cl100k_base tokenizer
pub fn get_token_count(text: &str) -> usize {
    get_tokenizer().encode_with_special_tokens(text).len()
}

/// Format token count with ~ prefix and "k" abbreviation
/// - Under 1k: plain number (e.g., "~500")
/// - 1k-9.9k: one decimal (e.g., "~1.1k")
/// - 10k+: no decimal (e.g., "~12k")
/// - 1m+: same pattern with "m"
pub fn format_token_count(count: usize) -> String {
    let formatted = if count < 1_000 {
        count.to_string()
    } else if count < 10_000 {
        let k = count as f64 / 1_000.0;
        format!("{k:.1}k")
    } else if count < 1_000_000 {
        let k = count / 1_000;
        format!("{k}k")
    } else if count < 10_000_000 {
        let m = count as f64 / 1_000_000.0;
        format!("{m:.1}m")
    } else {
        let m = count / 1_000_000;
        format!("{m}m")
    };

    format!("~{formatted}")
}
