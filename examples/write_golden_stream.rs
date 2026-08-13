//! golden fixture(既知値メッセージの `u32 LE 長さ + ペイロード` 連結ストリーム)を
//! ファイルへ書き出すだけの小さなヘルパ。SPEC §10.4 の「生成器」側。
//!
//! ```text
//! cargo run --quiet --example write_golden_stream -- /tmp/golden_stream.bin
//! ```
//!
//! 相方は C++ 側の検証器 `tools/root_sink/test_conformance.cpp`(本番デコーダで読む)。
//! 自動化は `tools/root_sink/run_conformance.sh`。
//! **生成物はコミットしない**(毎回再生成 —— 陳腐化が構造的に起きない)。

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path: PathBuf = std::env::args_os()
        .nth(1)
        .ok_or("usage: write_golden_stream <out.bin>")?
        .into();
    tpcdaq::msg::write_golden_stream(&path)?;
    println!("{}", path.display());
    Ok(())
}
