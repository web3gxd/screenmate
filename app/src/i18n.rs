//! User-facing language preference shared by the GUI and notification area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    Ru,
    En,
}

impl Language {
    pub fn text(self, ru: &'static str, en: &'static str) -> &'static str {
        match self { Self::Ru => ru, Self::En => en }
    }

    pub fn code(self) -> &'static str {
        match self { Self::Ru => "ru", Self::En => "en" }
    }

    pub fn from_code(code: &str) -> Self {
        if code.eq_ignore_ascii_case("en") { Self::En } else { Self::Ru }
    }
}
