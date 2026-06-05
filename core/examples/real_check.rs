//! Real-API check for the LLM path (translate + summarize) via the configured
//! summary provider (Groq by default, or Ollama/FastFlowLM if configured).
//! Run: GROQ_API_KEY=... cargo run -p tutanak-core --example real_check

use tutanak_core::{config::Config, engine};

fn main() -> anyhow::Result<()> {
    let cfg = Config::load()?;
    let llm = engine::build_llm(&cfg)?;
    println!("# özet motoru: {}", llm.label());

    let transcript = "Okay team, let's start the sprint planning. For this sprint we decided to \
        ship the new login page on Friday. Ahmet will fix the payment bug by Wednesday. \
        We still have an open question about the database migration timing, so let's revisit \
        that on Thursday. Also, Ayse raised a concern about the API rate limits.";

    println!("\n== Özet ({}) ==", cfg.output_lang);
    println!("{}\n", llm.summarize(transcript, &cfg.output_lang)?);

    println!("== Çeviri ({}) ==", cfg.output_lang);
    println!("{}", llm.translate(transcript, &cfg.output_lang)?);
    Ok(())
}
