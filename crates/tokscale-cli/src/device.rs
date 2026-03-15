use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[allow(dead_code)]
fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().context("Could not determine home directory")
}

#[allow(dead_code)]
fn get_device_id_path() -> Result<PathBuf> {
    Ok(home_dir()?.join(".config/tokscale/device-id"))
}

#[allow(dead_code)]
fn ensure_config_dir() -> Result<()> {
    let config_dir = home_dir()?.join(".config/tokscale");

    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&config_dir, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

#[allow(dead_code)]
pub fn get_or_create_device_id() -> Result<String> {
    ensure_config_dir()?;
    let path = get_device_id_path()?;

    if path.exists() {
        let id = fs::read_to_string(&path)?.trim().to_string();
        if !id.is_empty() {
            return Ok(id);
        }
    }

    let id = uuid::Uuid::new_v4().to_string();

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(id.as_bytes())?;
    }

    #[cfg(not(unix))]
    {
        fs::write(&path, &id)?;
    }

    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;
    use tempfile::TempDir;

    #[test]
    #[serial]
    fn test_get_device_id_path() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("HOME", temp_dir.path());
        }

        let path = get_device_id_path().unwrap();
        let expected = temp_dir.path().join(".config/tokscale/device-id");

        assert_eq!(path, expected);

        unsafe {
            env::remove_var("HOME");
        }
    }

    #[test]
    #[serial]
    fn test_get_or_create_device_id_creation() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("HOME", temp_dir.path());
        }

        let id = get_or_create_device_id().unwrap();
        assert!(!id.is_empty());

        // Verify it's a valid UUID v4 format (36 chars with dashes)
        assert_eq!(id.len(), 36);
        assert_eq!(id.matches('-').count(), 4);

        let path = get_device_id_path().unwrap();
        assert!(path.exists());

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, id);

        unsafe {
            env::remove_var("HOME");
        }
    }

    #[test]
    #[serial]
    fn test_get_or_create_device_id_idempotency() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("HOME", temp_dir.path());
        }

        let id1 = get_or_create_device_id().unwrap();
        let id2 = get_or_create_device_id().unwrap();

        assert_eq!(id1, id2);

        unsafe {
            env::remove_var("HOME");
        }
    }

    #[test]
    #[serial]
    #[cfg(unix)]
    fn test_get_or_create_device_id_permissions() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("HOME", temp_dir.path());
        }

        get_or_create_device_id().unwrap();

        let path = get_device_id_path().unwrap();
        let metadata = fs::metadata(&path).unwrap();
        let permissions = metadata.permissions();

        use std::os::unix::fs::PermissionsExt;
        assert_eq!(permissions.mode() & 0o777, 0o600);

        unsafe {
            env::remove_var("HOME");
        }
    }
}
