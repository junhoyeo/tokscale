use tempfile::TempDir;

pub fn temp_home() -> TempDir {
    tempfile::TempDir::new().expect("create integration-test home")
}
