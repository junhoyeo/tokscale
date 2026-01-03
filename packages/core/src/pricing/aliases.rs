use std::collections::HashMap;
use once_cell::sync::Lazy;

static MODEL_ALIASES: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("big-pickle", "glm-4.7");
    m.insert("big pickle", "glm-4.7");
    m.insert("bigpickle", "glm-4.7");

    // OpenAI Codex OAuth plugin models → Azure pricing (LiteLLM has azure/gpt-5.x)
    m.insert("gpt-5.2-codex-xhigh", "azure/gpt-5.2");
    m.insert("gpt-5.2-codex-high", "azure/gpt-5.2");
    m.insert("gpt-5.2-codex-medium", "azure/gpt-5.2");
    m.insert("gpt-5.2-codex", "azure/gpt-5.2");
    m.insert("gpt-5.2", "azure/gpt-5.2");
    m.insert("gpt-5.1-codex-xhigh", "azure/gpt-5.1");
    m.insert("gpt-5.1-codex-high", "azure/gpt-5.1");
    m.insert("gpt-5.1-codex-medium", "azure/gpt-5.1");
    m.insert("gpt-5.1-codex", "azure/gpt-5.1");
    m.insert("gpt-5.1", "azure/gpt-5.1");

    m
});

pub fn resolve_alias(model_id: &str) -> Option<&'static str> {
    MODEL_ALIASES.get(model_id.to_lowercase().as_str()).copied()
}
