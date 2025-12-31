use std::path::PathBuf;
use std::fs;
use std::time::SystemTime;
use serde::{Serialize, Deserialize};

const CACHE_TTL_SECS: u64 = 3600;

pub fn get_cache_dir() -> PathBuf {
    // Allow custom cache directory via environment variable
    if let Ok(custom_dir) = std::env::var("TOKSCALE_CACHE_DIR") {
        return PathBuf::from(custom_dir);
    }

    // On macOS and Linux, use ~/.cache/tokscale for consistency with CLI
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        if let Some(home) = dirs::home_dir() {
            return home.join(".cache").join("tokscale");
        }
    }

    // On Windows (or fallback), use platform defaults
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("tokscale")
}

pub fn get_cache_path(filename: &str) -> PathBuf {
    get_cache_dir().join(filename)
}

#[derive(Serialize, Deserialize)]
pub struct CachedData<T> {
    pub timestamp: u64,
    pub data: T,
}

pub fn load_cache<T: for<'de> Deserialize<'de>>(filename: &str) -> Option<T> {
    let path = get_cache_path(filename);
    let content = fs::read_to_string(&path).ok()?;
    let cached: CachedData<T> = serde_json::from_str(&content).ok()?;
    
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    
    if cached.timestamp > now || now.saturating_sub(cached.timestamp) > CACHE_TTL_SECS {
        return None;
    }
    
    Some(cached.data)
}

pub fn save_cache<T: Serialize>(filename: &str, data: &T) -> Result<(), std::io::Error> {
    let dir = get_cache_dir();
    fs::create_dir_all(&dir)?;
    
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    
    let cached = CachedData { timestamp: now, data };
    let content = serde_json::to_string(&cached)?;
    
    let final_path = get_cache_path(filename);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let tmp_filename = format!(".{}.{}.{:x}.tmp", filename, std::process::id(), nanos);
    let tmp_path = dir.join(&tmp_filename);
    
    use std::io::Write;
    let write_result = (|| {
        let mut file = fs::File::create(&tmp_path)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp_path, &final_path)
    })();
    
    if write_result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    
    write_result
}
