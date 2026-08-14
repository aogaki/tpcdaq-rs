//! `tpcdaq_state.json` の `next_run` 永続化(SPEC §8.1)。
//!
//! controller が単一書き手。ここは純粋な読み書きだけを提供する(§8.1 の REST 配線は 016)。
//!
//! # クラッシュ耐性(SPEC §12-11)
//!
//! [`take_next_run`] は「読む → `next_run + 1` を一時ファイルへ書いて `rename`(同一
//! ディレクトリ内の rename は POSIX で atomic)→ 読んだ値を返す」の順で進む。
//! **rename が成功して初めて値を返す**ので、rename 前にクラッシュしても状態ファイルは
//! 古いまま(次回同じ番号を読み直すだけ = 何も失われない)。rename 後・呼び出し元が
//! その値を使う前にクラッシュすると、その番号は**永久に飛ぶ**(SPEC §12-11 で明示的に許容)。
//! どちらの場合も**同じ番号が 2 回返ることはない**(rename 前に読み直した値は必ず
//! 前回 rename 済みの値以上になる)。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------
// エラー
// ---------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse state file {path}: {source}")]
    Decode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("failed to encode state: {0}")]
    Encode(#[from] serde_json::Error),

    #[error("next_run overflowed u32::MAX")]
    Overflow,
}

// ---------------------------------------------------------------------
// ファイル形式
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct StateFile {
    next_run: u32,
}

// ---------------------------------------------------------------------
// 内部ヘルパ
// ---------------------------------------------------------------------

fn io_err(path: &Path, source: std::io::Error) -> StateError {
    StateError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// 一時ファイルのパス(同一ディレクトリ、`.tmp` サフィックス — 同一ファイルシステム内の
/// `rename` を atomic にするため隣に置く)。
fn tmp_path_for(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(".tmp");
    PathBuf::from(os)
}

/// 現在の `next_run` を読む。ファイル不在は `1`(SPEC 015「ファイル不在時は 1 から」)。
fn read_next_run(path: &Path) -> Result<u32, StateError> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let state: StateFile =
                serde_json::from_str(&text).map_err(|source| StateError::Decode {
                    path: path.to_path_buf(),
                    source,
                })?;
            Ok(state.next_run)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(1),
        Err(e) => Err(io_err(path, e)),
    }
}

/// `next_run` を atomic に永続化する(一時ファイル書き込み → rename)。
fn persist_next_run(path: &Path, next_run: u32) -> Result<(), StateError> {
    let json = serde_json::to_string(&StateFile { next_run })?;
    let tmp_path = tmp_path_for(path);
    std::fs::write(&tmp_path, json.as_bytes()).map_err(|e| io_err(&tmp_path, e))?;
    std::fs::rename(&tmp_path, path).map_err(|e| io_err(path, e))?;
    Ok(())
}

// ---------------------------------------------------------------------
// 公開 API
// ---------------------------------------------------------------------

/// 現在の `next_run` を読み、`next_run + 1` を永続化してから、読んだ値を返す。
///
/// 永続化(rename)が成功して初めて値を返す。クラッシュで番号が飛ぶのは許容するが、
/// 同じ番号が重複することはない(SPEC §12-11)。ファイル不在時は `1` から。
pub fn take_next_run(path: impl AsRef<Path>) -> Result<u32, StateError> {
    let path = path.as_ref();
    let current = read_next_run(path)?;
    let next = current.checked_add(1).ok_or(StateError::Overflow)?;
    persist_next_run(path, next)?;
    Ok(current)
}

/// REST からの手動設定用(016 が audit 付きで呼ぶ)。`next_run` を `n` に上書きする。
pub fn set_next_run(path: impl AsRef<Path>, n: u32) -> Result<(), StateError> {
    persist_next_run(path.as_ref(), n)
}

// ---------------------------------------------------------------------
// テスト(仕様書 — 先に書く)
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// テスト用に一意な一時ディレクトリ + `tpcdaq_state.json` パスを作る。
    fn temp_state_path(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "tpcdaq-state-test-{tag}-{}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("tpcdaq_state.json")
    }

    #[test]
    fn missing_file_starts_at_1_and_persists_2() {
        let path = temp_state_path("missing-starts-at-1");
        let _ = std::fs::remove_file(&path);

        let run = take_next_run(&path).unwrap();
        assert_eq!(run, 1);

        let text = std::fs::read_to_string(&path).unwrap();
        let state: StateFile = serde_json::from_str(&text).unwrap();
        assert_eq!(state.next_run, 2);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn consecutive_calls_return_sequential_values() {
        let path = temp_state_path("sequential");
        let _ = std::fs::remove_file(&path);

        assert_eq!(take_next_run(&path).unwrap(), 1);
        assert_eq!(take_next_run(&path).unwrap(), 2);
        assert_eq!(take_next_run(&path).unwrap(), 3);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// プロセス再起動を模す: 呼び出しのたびに新しくファイルを読み直す(＝関数はステートレス)。
    #[test]
    fn value_survives_across_separate_reads_like_a_process_restart() {
        let path = temp_state_path("restart");
        let _ = std::fs::remove_file(&path);

        assert_eq!(take_next_run(&path).unwrap(), 1);
        // ここで「プロセスが死んで再起動した」体で、同じパスに対し新たに呼ぶだけ。
        assert_eq!(take_next_run(&path).unwrap(), 2);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn set_next_run_overrides_and_take_next_run_returns_it() {
        let path = temp_state_path("manual-set");
        let _ = std::fs::remove_file(&path);

        // 非対称な値(既定の 1 から始まる連番と衝突しない)。
        set_next_run(&path, 777).unwrap();
        assert_eq!(take_next_run(&path).unwrap(), 777);
        assert_eq!(take_next_run(&path).unwrap(), 778);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn atomic_write_leaves_no_tmp_file_behind() {
        let path = temp_state_path("no-tmp-leftover");
        let _ = std::fs::remove_file(&path);

        take_next_run(&path).unwrap();

        assert!(path.exists());
        assert!(!tmp_path_for(&path).exists());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// 025: `set_next_run` も `take_next_run` と同じ atomic-rename 規律(一時ファイル →
    /// rename)であることを、ファイル内容を直接読み直して確認する(`take_next_run` を
    /// 経由しない — テストの対象を持ち越さない独立照合)。
    #[test]
    fn set_next_run_is_atomic_and_the_persisted_value_survives_a_reread() {
        let path = temp_state_path("set-next-run-atomic");
        let _ = std::fs::remove_file(&path);

        set_next_run(&path, 4242).unwrap();

        assert!(path.exists());
        assert!(!tmp_path_for(&path).exists(), "rename 後は .tmp が残らない");

        // 再読込(プロセス再起動を模す): ファイルを直接パースして値が生き残っていること。
        let text = std::fs::read_to_string(&path).unwrap();
        let state: StateFile = serde_json::from_str(&text).unwrap();
        assert_eq!(state.next_run, 4242);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    // -----------------------------------------------------------------
    // 耐久(SPEC §12-11 の再現)
    // -----------------------------------------------------------------

    /// ②`take_next_run` を「rename 直後にクラッシュ」相当(値を使わず捨てる)で繰り返す。
    /// 番号は飛んでよいが、返り値は狭義単調増加(＝重複ゼロ)。
    #[test]
    fn repeated_crash_after_rename_never_duplicates_a_run_number() {
        let path = temp_state_path("crash-after-rename");
        let _ = std::fs::remove_file(&path);

        let mut used = Vec::new();
        for _ in 0..50 {
            // 「rename 直後にクラッシュ」= 戻り値を実際には使わずに捨てる、を模す。
            let run = take_next_run(&path).unwrap();
            used.push(run); // 捨てたはずの値も記録だけはして、あとで重複を検証する。
        }

        // 重複ゼロ = 集合にしても要素数が変わらない。
        let mut sorted = used.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), used.len(), "duplicate run numbers: {used:?}");

        // 狭義単調増加(番号が飛ぶのは可、逆転・重複は不可)。
        for pair in used.windows(2) {
            assert!(pair[1] > pair[0], "not strictly increasing: {used:?}");
        }

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn corrupt_state_file_is_a_decode_error_not_a_silent_reset() {
        let path = temp_state_path("corrupt");
        std::fs::write(&path, b"not json").unwrap();

        let err = take_next_run(&path).unwrap_err();
        assert!(matches!(err, StateError::Decode { .. }));

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
