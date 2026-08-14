//! ws_proto_sample — WS プロトコルの適合性テスト用サンプル生成器(SPEC §10.4-1、TODO/026)。
//!
//! 使い方: `ws_proto_sample --out <path>`
//!
//! **本番エンコーダ**([`tpcdaq::monitor::ws`])で全メッセージ型(0x02/0x03/0x10/0x11)を
//! 既知値でエンコードし、`u32 LE 長さ + ペイロード` の連結ストリームをファイルへ書く。
//! 検証器は TypeScript 側(Angular UI の**本番デコーダ**を import するテスト、027)。
//!
//! 出力は決定的(同一入力 → 同一バイト)。**生成物はコミットしない**
//! ——毎回再生成することで陳腐化が構造的に起きない(SPEC §10.4-3)。

use std::path::PathBuf;
use std::process::ExitCode;

use tpcdaq::monitor::write_ws_sample_stream;

const USAGE: &str = "usage: ws_proto_sample --out <path>";

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let out = match parse_args(&raw) {
        Ok(out) => out,
        Err(message) => {
            eprintln!("ws_proto_sample: {message}");
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    match write_ws_sample_stream(&out) {
        Ok(()) => {
            println!("{}", out.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("ws_proto_sample: cannot write {}: {e}", out.display());
            ExitCode::FAILURE
        }
    }
}

/// 手書きの引数パース(追加依存なし。他の bin と同じ流儀)。
fn parse_args(raw: &[String]) -> Result<PathBuf, String> {
    let mut out: Option<PathBuf> = None;

    let mut iter = raw.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--out" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--out requires a path".to_string())?;
                out = Some(PathBuf::from(value));
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    out.ok_or_else(|| "--out is required".to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::parse_args;
    use std::path::PathBuf;

    fn strs(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_the_output_path() {
        let out = parse_args(&strs(&["--out", "/tmp/ws_sample.bin"])).unwrap();
        assert_eq!(out, PathBuf::from("/tmp/ws_sample.bin"));
    }

    #[test]
    fn rejects_missing_or_malformed_arguments() {
        assert!(parse_args(&strs(&[])).is_err());
        assert!(parse_args(&strs(&["--out"])).is_err());
        assert!(parse_args(&strs(&["--bogus", "x"])).is_err());
    }
}
