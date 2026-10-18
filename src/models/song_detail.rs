use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricLine {
    pub time: Duration,
    pub text: String,
    pub translation: Option<String>,
}
