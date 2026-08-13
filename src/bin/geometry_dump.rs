//! `geometry_dump <file.dat>` — `geometry::dump_tsv` を stdout に出すだけの薄い CLI。
//!
//! ```text
//! cargo run --quiet --bin geometry_dump -- tests/fixtures/geometry_mini_reduced.dat
//! ```
//!
//! 相方は C++ 側の `tools/root_sink/test_geo`(`test_geo <file.dat>` のダンプモード)。
//! 二重実装の一致は `tools/root_sink/run_geo_conformance.sh` がバイト一致で検証する
//! (SPEC §4.5、TODO/018)。既存モジュール(`tpcdaq::geometry`)を呼ぶだけで、
//! `src/lib.rs` の改変は不要。

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path: PathBuf = std::env::args_os()
        .nth(1)
        .ok_or("usage: geometry_dump <file.dat>")?
        .into();
    let geometry = tpcdaq::geometry::load(&path)?;
    // `dump_tsv` は各行を `writeln!` で組み立てるため、返り値は既に末尾 "\n" 込み。
    // `println!` を使うと空行が 1 行余分に付くので `print!` にする。
    print!("{}", tpcdaq::geometry::dump_tsv(&geometry));
    Ok(())
}
