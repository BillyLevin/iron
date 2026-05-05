use std::{
    fmt,
    path::Path,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Language {
    Rust,
    Toml,
    Text,
}

impl Language {
    pub(crate) fn new(file_path: &Path) -> Self {
        file_path
            .extension()
            .and_then(|ext| ext.to_str())
            .and_then(Self::from_extension)
            .or_else(|| {
                file_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(Self::from_file_name)
            })
            .unwrap_or(Self::Text)
    }

    fn from_extension(extension: &str) -> Option<Self> {
        match extension {
            "rs" => Some(Self::Rust),
            "toml" => Some(Self::Toml),
            _ => None,
        }
    }

    fn from_file_name(file_name: &str) -> Option<Self> {
        match file_name {
            "Cargo.lock" => Some(Self::Toml),
            _ => None,
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", match *self {
            Self::Rust => "rust",
            Self::Toml => "toml",
            Self::Text => "text",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn by_extension() {
        assert_eq!(Language::new(Path::new("foo/file.rs")), Language::Rust);
        assert_eq!(Language::new(Path::new("Cargo.toml")), Language::Toml);
        assert_eq!(Language::new(Path::new("thing")), Language::Text);
    }

    #[test]
    fn by_file_name() {
        assert_eq!(Language::new(Path::new("Cargo.lock")), Language::Toml);
        assert_eq!(Language::new(Path::new("Cargo.lockk")), Language::Text);
    }
}
