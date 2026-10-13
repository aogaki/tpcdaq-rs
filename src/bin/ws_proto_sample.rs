//! ws_proto_sample — WS プロトコルの適合性テスト用サンプル生成器(SPEC §10.4-1、TODO/026)。
//!
//! 使い方: `ws_proto_sample --out <path> [--geometry <dat> --strip-geometry-out <json>]`
//!
//! **本番エンコーダ**([`tpcdaq::monitor::ws`])で全メッセージ型(0x02/0x03/0x10/0x11)を
//! 既知値でエンコードし、`u32 LE 長さ + ペイロード` の連結ストリームをファイルへ書く。
//! 検証器は TypeScript 側(Angular UI の**本番デコーダ**を import するテスト、027)。
//!
//! 後ろ 2 つを付けると、ジオメトリ .dat から monitor が配る `stripGeometry` JSON
//! (`strip_geometry_json` of `strip_xy::strip_geometry`)も書く(TODO/125 レーン C の照合入力)。
//!
//! 出力は決定的(同一入力 → 同一バイト)。**生成物はコミットしない**
//! ——毎回再生成することで陳腐化が構造的に起きない(SPEC §10.4-3)。

use std::path::PathBuf;
use std::process::ExitCode;

use tpcdaq::monitor::{strip_geometry_json, write_ws_sample_stream};
use tpcdaq::{geometry, strip_xy};

const USAGE: &str =
    "usage: ws_proto_sample --out <path> [--geometry <dat> --strip-geometry-out <json>]";

/// 解析済みの引数。
#[derive(Debug, PartialEq)]
struct Args {
    out: PathBuf,
    /// `(ジオメトリ .dat, stripGeometry JSON の出力先)`。
    strip_geometry: Option<(PathBuf, PathBuf)>,
}

/// monitor が配るのと同じ `stripGeometry` JSON をファイルへ書く。
fn write_strip_geometry(dat: &PathBuf, out: &PathBuf) -> Result<(), String> {
    let loaded = geometry::load(dat).map_err(|e| format!("cannot load {}: {e}", dat.display()))?;
    let built = strip_xy::strip_geometry(&loaded).map(|(g, _)| g);
    let text = strip_geometry_json(&built).map_err(|e| format!("cannot encode: {e}"))?;
    std::fs::write(out, text).map_err(|e| format!("cannot write {}: {e}", out.display()))
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&raw) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("ws_proto_sample: {message}");
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    if let Err(e) = write_ws_sample_stream(&args.out) {
        eprintln!("ws_proto_sample: cannot write {}: {e}", args.out.display());
        return ExitCode::FAILURE;
    }
    println!("{}", args.out.display());

    if let Some((dat, json)) = &args.strip_geometry {
        if let Err(message) = write_strip_geometry(dat, json) {
            eprintln!("ws_proto_sample: {message}");
            return ExitCode::FAILURE;
        }
        println!("{}", json.display());
    }
    ExitCode::SUCCESS
}

/// 手書きの引数パース(追加依存なし。他の bin と同じ流儀)。
fn parse_args(raw: &[String]) -> Result<Args, String> {
    let mut out: Option<PathBuf> = None;
    let mut dat: Option<PathBuf> = None;
    let mut json: Option<PathBuf> = None;

    let mut iter = raw.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--out" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--out requires a path".to_string())?;
                out = Some(PathBuf::from(value));
            }
            "--geometry" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--geometry requires a path".to_string())?;
                dat = Some(PathBuf::from(value));
            }
            "--strip-geometry-out" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--strip-geometry-out requires a path".to_string())?;
                json = Some(PathBuf::from(value));
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    let strip_geometry = match (dat, json) {
        (Some(dat), Some(json)) => Some((dat, json)),
        (None, None) => None,
        _ => return Err("--geometry and --strip-geometry-out go together".to_string()),
    };
    let out = out.ok_or_else(|| "--out is required".to_string())?;
    Ok(Args {
        out,
        strip_geometry,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{parse_args, Args};
    use std::path::PathBuf;

    fn strs(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_the_output_path() {
        let args = parse_args(&strs(&["--out", "/tmp/ws_sample.bin"])).unwrap();
        assert_eq!(
            args,
            Args {
                out: PathBuf::from("/tmp/ws_sample.bin"),
                strip_geometry: None,
            }
        );
    }

    #[test]
    fn parses_the_strip_geometry_pair_and_rejects_half_of_it() {
        let args = parse_args(&strs(&[
            "--out",
            "a.bin",
            "--geometry",
            "g.dat",
            "--strip-geometry-out",
            "g.json",
        ]))
        .unwrap();
        assert_eq!(
            args.strip_geometry,
            Some((PathBuf::from("g.dat"), PathBuf::from("g.json")))
        );
        assert!(parse_args(&strs(&["--out", "a.bin", "--geometry", "g.dat"])).is_err());
        assert!(parse_args(&strs(&["--out", "a.bin", "--strip-geometry-out", "g.json"])).is_err());
    }

    #[test]
    fn rejects_missing_or_malformed_arguments() {
        assert!(parse_args(&strs(&[])).is_err());
        assert!(parse_args(&strs(&["--out"])).is_err());
        assert!(parse_args(&strs(&["--bogus", "x"])).is_err());
    }
}
