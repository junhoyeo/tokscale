pub(crate) fn looks_like_email(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.contains('@') && trimmed.split('@').count() == 2
}
