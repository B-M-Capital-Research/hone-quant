//! The AI sector view: once a trading day, before the open, a language model (honeclaw's Gemini
//! route) reads honeclaw's ontology research and objective data and proposes a bounded stance per
//! sector. The strategy can ignore it, record what it would have done (shadow) or apply it (live);
//! see `quant_core::strategy::AiMode`.
//!
//! - [`llm`]: the OpenAI-compatible client.
//! - [`material`]: the briefing: ontology digest, market statistics, earnings against consensus.
//! - [`view`]: the prompt and the strict validation of the answer.

pub mod llm;
pub mod material;
pub mod view;
