//! the modal pickers and their filters.

//! Terminal UI state and rendering for exp-1 agent.
//!
//! Layout follows the OpenAI Codex TUI: a `>_` masthead, submitted prompts as
//! highlighted bands in the transcript, a completion popup or selection picker
//! above the input, a padded input band with a blinking caret, then a model line
//! and a hint line. Two threads exist — the main conversation and a side thread
//! opened with ctrl+/.


#[derive(Clone)]
pub struct PickerItem {
    pub label: String,
    pub description: String,
}

#[derive(Clone, Copy, PartialEq)]
pub enum PickerKind {
    Model,
    Effort,
    Theme,
    Session,
    Provider,
    Scoped,
    /// What to do with one of your own messages: copy it, or go back to it.
    Message,
}

impl Picker {
    /// Rows the panel wants: title, spacers, one per item, hints.
    pub fn height(&self) -> u16 {
        self.items.len() as u16 + 5
    }
}

#[derive(Clone)]
pub struct Picker {
    pub kind: PickerKind,
    pub title: String,
    pub items: Vec<PickerItem>,
    pub selected: usize,
}
/// The effort to use after switching to a model that takes `levels`: the one in
/// use when it is among them, its stated default, or the first it offers. A
/// model with no levels takes whatever it is given.
pub fn settle_effort(levels: &[String], default: Option<&str>, current: &str) -> String {
    if levels.is_empty() || levels.iter().any(|level| level == current) {
        return current.to_string();
    }
    default
        .filter(|default| levels.iter().any(|level| level == default))
        .map(str::to_string)
        .unwrap_or_else(|| levels[0].clone())
}

/// The key a scope entry uses for one model. The provider is part of it,
/// because the same name can be offered by two of them and ticking one must not
/// tick the other.
pub fn scope_key(model: &crate::proto::WireModel) -> String {
    if model.provider.is_empty() {
        model.slug.clone()
    } else {
        format!("{}/{}", model.provider, model.slug)
    }
}
