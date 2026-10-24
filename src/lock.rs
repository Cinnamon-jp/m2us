//! 二重起動防止のためのファイルロック（OS非依存・テスト可能）。
//!
//! `flock` によるロックは、プロセスが終了（クラッシュ・SIGKILL含む）するとOSが自動解放する。

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

/// ロックファイルの既定パス。macOSでは `TMPDIR` がユーザーごとのディレクトリになる。
pub fn default_path() -> PathBuf {
    std::env::temp_dir().join("m2us.lock")
}

/// ロックを取得する。返された `File` を保持している間ロックが有効になる。
pub fn acquire(path: &Path) -> Result<File, &'static str> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|_| "Failed to open the lock file.")?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err("m2us is already running."),
        Err(TryLockError::Error(_)) => Err("Failed to acquire the instance lock."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("m2us-test-{}-{}.lock", std::process::id(), name))
    }

    #[test]
    fn test_second_acquire_fails_while_held() {
        let path = test_path("held");
        let first = acquire(&path).expect("first acquire should succeed");
        assert_eq!(acquire(&path).err(), Some("m2us is already running."));
        drop(first);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_acquire_succeeds_after_release() {
        let path = test_path("released");
        drop(acquire(&path).expect("first acquire should succeed"));
        let again = acquire(&path);
        assert!(again.is_ok());
        drop(again);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_different_paths_do_not_conflict() {
        let a = test_path("a");
        let b = test_path("b");
        let _la = acquire(&a).unwrap();
        let _lb = acquire(&b).unwrap();
        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }

    #[test]
    fn test_unopenable_path_is_reported() {
        let path = std::env::temp_dir()
            .join("m2us-nonexistent-dir-for-test")
            .join("x.lock");
        assert_eq!(acquire(&path).err(), Some("Failed to open the lock file."));
    }
}
