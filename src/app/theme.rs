//! the palettes.

//! Terminal UI state and rendering for exp-1 agent.
//!
//! Layout follows the OpenAI Codex TUI: a `>_` masthead, submitted prompts as
//! highlighted bands in the transcript, a completion popup or selection picker
//! above the input, a padded input band with a blinking caret, then a model line
//! and a hint line. Two threads exist — the main conversation and a side thread
//! opened with ctrl+/.
use ratatui::style::{ Color };


/// A colour scheme. Colours are truecolor so a terminal theme cannot remap them.
pub struct Theme {
    pub name: &'static str,
    pub accent: Color,
    pub dim: Color,
    pub selection: Color,
    pub input_bg: Color,
    pub warn: Color,
    pub code: Color,
    pub math: Color,
}

pub(crate) const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

pub const THEMES: [Theme; 4] = [
    Theme {
        name: "default",
        accent: rgb(59, 130, 246),
        dim: rgb(120, 120, 120),
        selection: rgb(59, 130, 246),
        input_bg: rgb(58, 58, 58),
        warn: rgb(239, 111, 111),
        code: rgb(126, 200, 140),
        math: rgb(214, 178, 96),
    },
    Theme {
        name: "amber",
        accent: rgb(217, 145, 32),
        dim: rgb(133, 122, 100),
        selection: rgb(217, 145, 32),
        input_bg: rgb(56, 48, 34),
        warn: rgb(220, 120, 90),
        code: rgb(150, 205, 160),
        math: rgb(240, 200, 110),
    },
    Theme {
        name: "jade",
        accent: rgb(52, 190, 140),
        dim: rgb(110, 130, 118),
        selection: rgb(52, 190, 140),
        input_bg: rgb(36, 52, 45),
        warn: rgb(235, 150, 120),
        code: rgb(120, 205, 160),
        math: rgb(200, 225, 140),
    },
    Theme {
        name: "mono",
        accent: rgb(214, 214, 214),
        dim: rgb(122, 122, 122),
        selection: rgb(84, 88, 96),
        input_bg: rgb(50, 50, 50),
        warn: rgb(220, 130, 130),
        code: rgb(180, 180, 180),
        math: rgb(210, 210, 210),
    },
];
