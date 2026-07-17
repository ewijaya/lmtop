use serde::{Deserialize, Serialize};

/// Coarse model grouping used for colors, breakdown rows, and filtering.
///
/// This is deliberately open-ended: unknown or future models keep their raw
/// identifier and are grouped under `Other`, never dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFamily {
    ClaudeFable,
    ClaudeOpus,
    ClaudeSonnet,
    ClaudeHaiku,
    Gpt,
    Other,
}

impl ModelFamily {
    pub fn label(self) -> &'static str {
        match self {
            ModelFamily::ClaudeFable => "Fable",
            ModelFamily::ClaudeOpus => "Opus",
            ModelFamily::ClaudeSonnet => "Sonnet",
            ModelFamily::ClaudeHaiku => "Haiku",
            ModelFamily::Gpt => "GPT",
            ModelFamily::Other => "Other",
        }
    }
}

/// A model as seen in provider data: the raw identifier is always preserved;
/// `display` is a normalized human-readable name; `family` groups related
/// versions together.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ModelIdentity {
    /// Raw identifier exactly as reported by the provider
    /// (e.g. `claude-opus-4-8`, `gpt-5.2-codex`).
    pub raw: String,
    /// Normalized display name (e.g. `Opus 4.8`, `gpt-5.2-codex`).
    pub display: String,
    pub family: ModelFamily,
}

impl ModelIdentity {
    /// Normalize a raw model identifier. Works for both providers and keeps
    /// unknown identifiers intact under `ModelFamily::Other`.
    pub fn normalize(raw: &str) -> ModelIdentity {
        let lower = raw.to_ascii_lowercase();
        let (family, display) = if let Some(rest) = lower.strip_prefix("claude-") {
            classify_claude(rest)
        } else if lower.starts_with("gpt-")
            || lower.starts_with("o1")
            || lower.starts_with("o3")
            || lower.starts_with("o4")
            || lower.starts_with("codex")
        {
            (ModelFamily::Gpt, raw.to_string())
        } else if lower.contains("fable") {
            (ModelFamily::ClaudeFable, claude_display("Fable", &lower))
        } else if lower.contains("opus") {
            (ModelFamily::ClaudeOpus, claude_display("Opus", &lower))
        } else if lower.contains("sonnet") {
            (ModelFamily::ClaudeSonnet, claude_display("Sonnet", &lower))
        } else if lower.contains("haiku") {
            (ModelFamily::ClaudeHaiku, claude_display("Haiku", &lower))
        } else {
            (ModelFamily::Other, raw.to_string())
        };
        ModelIdentity {
            raw: raw.to_string(),
            display,
            family,
        }
    }
}

fn classify_claude(rest: &str) -> (ModelFamily, String) {
    let family = if rest.contains("fable") {
        ModelFamily::ClaudeFable
    } else if rest.contains("opus") {
        ModelFamily::ClaudeOpus
    } else if rest.contains("sonnet") {
        ModelFamily::ClaudeSonnet
    } else if rest.contains("haiku") {
        ModelFamily::ClaudeHaiku
    } else {
        ModelFamily::Other
    };
    let display = match family {
        ModelFamily::Other => format!("claude-{rest}"),
        f => claude_display(f.label(), rest),
    };
    (family, display)
}

/// Build `"<Family> <version>"` from segments like `opus-4-8-20250801` or
/// `fable-5`. Trailing date-like segments (8 digits) are dropped.
fn claude_display(family_label: &str, rest: &str) -> String {
    let mut version_parts: Vec<&str> = Vec::new();
    for seg in rest.split(['-', '.']) {
        if seg.chars().all(|c| c.is_ascii_digit()) {
            if seg.len() >= 8 {
                break; // date stamp, not a version component
            }
            version_parts.push(seg);
        }
    }
    if version_parts.is_empty() {
        family_label.to_string()
    } else {
        format!("{family_label} {}", version_parts.join("."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_fable() {
        let m = ModelIdentity::normalize("claude-fable-5");
        assert_eq!(m.family, ModelFamily::ClaudeFable);
        assert_eq!(m.display, "Fable 5");
        assert_eq!(m.raw, "claude-fable-5");
    }

    #[test]
    fn normalizes_dated_opus() {
        let m = ModelIdentity::normalize("claude-opus-4-8-20250801");
        assert_eq!(m.family, ModelFamily::ClaudeOpus);
        assert_eq!(m.display, "Opus 4.8");
    }

    #[test]
    fn normalizes_sonnet_and_haiku() {
        assert_eq!(
            ModelIdentity::normalize("claude-sonnet-5").family,
            ModelFamily::ClaudeSonnet
        );
        let h = ModelIdentity::normalize("claude-haiku-4-5-20251001");
        assert_eq!(h.family, ModelFamily::ClaudeHaiku);
        assert_eq!(h.display, "Haiku 4.5");
    }

    #[test]
    fn keeps_unknown_models_intact() {
        let m = ModelIdentity::normalize("claude-nebula-9");
        assert_eq!(m.family, ModelFamily::Other);
        assert_eq!(m.raw, "claude-nebula-9");
        let m = ModelIdentity::normalize("totally-new-model-x");
        assert_eq!(m.family, ModelFamily::Other);
        assert_eq!(m.display, "totally-new-model-x");
    }

    #[test]
    fn classifies_gpt_models() {
        assert_eq!(
            ModelIdentity::normalize("gpt-5.2-codex").family,
            ModelFamily::Gpt
        );
    }
}
