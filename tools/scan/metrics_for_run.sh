#!/usr/bin/env bash
#
# TODO/136 — run 1 本 → 指標 JSON。
#
#   tools/scan/metrics_for_run.sh [--macros DIR] [--pressure MBAR] [--ped FILE [--zs-fill]] RUN_DIR OUT_DIR
#
#   RUN_DIR     root-sink の run ディレクトリ(`<output_root>/run0077/`)。中の `run0077_raw.root` を読む
#   OUT_DIR     出力先。`run0077_metrics.json` と、macros が作る `run0077_uvw.root` /
#               `run0077_tracks.root` をここに置く(RUN_DIR = データ置き場は汚さない — raw は
#               OUT_DIR へのシンボリックリンクで macros に渡す。macros は名前の `_raw.root` を
#               置き換えて出力するので、出力はリンクの隣 = OUT_DIR に出る)
#   --macros    analyzeUVW.C / makeTracks.C / channel_map.csv のあるディレクトリ
#               (既定 ~/test/ana/ana_107。読むだけ・そこで実行するだけで、書き換えない)
#   --pressure  analyzeUVW.C の第 2 引数 [mbar](ドリフト速度 = z の換算。既定 100 = macros の既定)
#
#   --ped       `calib pedestal` の JSON(TODO/149)。ZS 取得のデータ用: ped = 表の mean、FPN 共通モードは引かない。
#               無ければ従来どおり(cells 1..20 の ped + FPN 減算。ZS の疑いがあれば WARNING)
#   --zs-fill   (--ped 必須、TODO/154)ZS の raw の抑制セルを表の round(mean) で埋めた raw を
#               `OUT_DIR/zsfilled/runNNNN_raw.root`(同名 — macros は `_raw.root` → `_uvw.root` の置換で出力名を
#               作る)に zs_restore.C で作り、macros(無改造)と run_metrics --ped はそれを読む。
#               `_uvw.root` / `_tracks.root` も zsfilled/ に出る。指標 JSON は OUT_DIR(`raw_file` = 復元パス)。
#               zsfilled/ に復元済みの raw があれば作り直さない(--ped を変えたら zsfilled/ を消す)
#
# `_tracks.root` が OUT_DIR に無ければ macros で作る(`_uvw.root` も無ければ analyzeUVW.C から)。
# macros が無い・tracks が作れなかったときは WARNING を出して tracks = null のまま raw の指標だけ出す。
# 終了コード: 0 = JSON を書いた、1 = 入力が無い・run_metrics.C が失敗、2 = 使い方の誤り。
set -euo pipefail

usage() {
  echo "usage: $0 [--macros DIR] [--pressure MBAR] [--ped FILE [--zs-fill]] RUN_DIR OUT_DIR" >&2
  exit 2
}

macros="${HOME}/test/ana/ana_107"
pressure=100
ped=""
zs_fill=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --macros) [[ $# -ge 2 ]] || usage; macros="$2"; shift 2 ;;
    --pressure) [[ $# -ge 2 ]] || usage; pressure="$2"; shift 2 ;;
    --ped) [[ $# -ge 2 ]] || usage; ped="$2"; shift 2 ;;
    --zs-fill) zs_fill=1; shift ;;
    -h | --help) usage ;;
    -*) echo "unknown option: $1" >&2; usage ;;
    *) break ;;
  esac
done
[[ $# -eq 2 ]] || usage
if [[ "${zs_fill}" == 1 && -z "${ped}" ]]; then
  echo "ERROR: --zs-fill needs --ped FILE" >&2
  exit 2
fi

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
run_dir="$(cd "$1" && pwd)"
run="$(basename "${run_dir}")"
raw="${run_dir}/${run}_raw.root"
if [[ ! -f "${raw}" ]]; then
  echo "ERROR: ${raw} not found (RUN_DIR must be <output_root>/runNNNN/)" >&2
  exit 1
fi
out_dir="$(mkdir -p "$2" && cd "$2" && pwd)"
json="${out_dir}/${run}_metrics.json"
# macros の出力(_uvw / _tracks)は入力 raw の隣に出る。--zs-fill では復元 raw のある zsfilled/ が作業場所
work_dir="${out_dir}"
[[ "${zs_fill}" == 1 ]] && work_dir="${out_dir}/zsfilled"
link="${work_dir}/${run}_raw.root"
uvw="${work_dir}/${run}_uvw.root"
tracks="${work_dir}/${run}_tracks.root"

if [[ "${zs_fill}" == 1 ]]; then
  :  # raw は後で復元して置く(シンボリックリンクは作らない)
elif [[ -e "${link}" || -L "${link}" ]]; then
  if [[ ! "${link}" -ef "${raw}" ]]; then
    echo "ERROR: ${link} exists and is not ${raw}" >&2
    exit 1
  fi
else
  ln -s "${raw}" "${link}"
fi

# macOS(CLT 27)では cling が libc++ のヘッダを見失う — macros/README.md の SDKROOT 注記。
if [[ -z "${SDKROOT:-}" ]] && command -v xcrun >/dev/null 2>&1; then
  SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
  export SDKROOT
fi
root_bin="$(command -v root || echo /opt/ROOT/bin/root)"

if [[ "${zs_fill}" == 1 ]]; then
  [[ -f "${ped}" ]] || { echo "ERROR: --ped ${ped} not found" >&2; exit 1; }
  ped_abs="$(cd "$(dirname "${ped}")" && pwd)/$(basename "${ped}")"
  mkdir -p "${work_dir}"
  if [[ -f "${link}" ]]; then
    echo "zs-fill: using existing ${link} (made earlier — delete ${work_dir} to redo)"
  else
    echo "zs-fill: zs_restore.C -> ${link}"
    "${root_bin}" -l -b -q "${here}/zs_restore.C(\"${raw}\", \"${ped_abs}\", \"${link}\")" ||
      { echo "ERROR: zs_restore.C failed" >&2; rm -f "${link}"; exit 1; }
    [[ -f "${link}" ]] || { echo "ERROR: ${link} was not produced" >&2; exit 1; }
  fi
fi

if [[ -f "${tracks}" ]]; then
  echo "tracks: using existing ${tracks} (made earlier — --pressure is not re-applied; delete it to redo)"
elif [[ ! -f "${macros}/analyzeUVW.C" || ! -f "${macros}/makeTracks.C" ||
  ! -f "${macros}/channel_map.csv" ]]; then
  echo "WARNING: ${macros} lacks analyzeUVW.C / makeTracks.C / channel_map.csv — tracks = null" >&2
else
  # analyzeUVW.C は channel_map.csv をカレントディレクトリから読む → macros の中で実行する
  if [[ ! -f "${uvw}" ]]; then
    echo "tracks: analyzeUVW.C (${pressure} mbar) -> ${uvw}"
    (cd "${macros}" && "${root_bin}" -l -b -q "analyzeUVW.C(\"${link}\", ${pressure})") ||
      echo "WARNING: analyzeUVW.C exited non-zero" >&2
  fi
  if [[ -f "${uvw}" ]]; then
    echo "tracks: makeTracks.C -> ${tracks}"
    (cd "${macros}" && "${root_bin}" -l -b -q "makeTracks.C(\"${uvw}\")") ||
      echo "WARNING: makeTracks.C exited non-zero" >&2
  fi
  [[ -f "${tracks}" ]] || echo "WARNING: ${tracks} was not produced — tracks = null" >&2
fi

if [[ -n "${ped}" ]]; then
  [[ -f "${ped}" ]] || { echo "ERROR: --ped ${ped} not found" >&2; exit 1; }
  ped="$(cd "$(dirname "${ped}")" && pwd)/$(basename "${ped}")"
  "${root_bin}" -l -b -q "${here}/run_metrics.C(\"${link}\", \"${json}\", ${pressure}, \"${ped}\")"
else
  "${root_bin}" -l -b -q "${here}/run_metrics.C(\"${link}\", \"${json}\", ${pressure})"
fi
