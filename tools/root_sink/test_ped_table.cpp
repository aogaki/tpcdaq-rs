// test_ped_table.cpp — ped_table.hpp の単体テスト(ROOT 不要・ZMQ 不要)。TODO/151。
//
//   g++ -std=c++17 -O2 -Wall -Wextra test_ped_table.cpp -o test_ped_table && ./test_ped_table
//
// 読む JSON は `calib pedestal`(src/calib/pedestal.rs)が serde_json::to_string_pretty で
// 書く形そのもの: `{meta:{inputs, events, generated_at, tool_version},
// channels:[{asad, aget, ch, mean, rms, runs, is_fpn}]}`。テストはその形の小さな表を
// 使い捨てディレクトリに書いてから読む。

#include <unistd.h>

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <string>

#include "check.hpp"
#include "ped_table.hpp"

namespace {

// double の厳密比較(許容差 0)。落ちたときに実値が出る。
#define CHECK_D(actual, expected)                                                     \
  do {                                                                                \
    const double a_ = (actual);                                                       \
    const double e_ = (expected);                                                     \
    if (a_ == e_) {                                                                   \
      ++tpccheck::g_pass;                                                             \
    } else {                                                                          \
      ++tpccheck::g_fail;                                                             \
      std::printf("FAIL %s:%d  %s == %s (got %.17g, want %.17g)\n", __FILE__,         \
                  __LINE__, #actual, #expected, a_, e_);                              \
    }                                                                                 \
  } while (0)

// 使い捨てファイル(固定パスを書かない = 並列実行・再実行に耐える)。
std::string scratch_file(const char* tag, const std::string& text) {
  const char* tmp = std::getenv("TMPDIR");
  std::string base = (tmp != nullptr && tmp[0] != '\0') ? std::string(tmp) : std::string("/tmp");
  if (!base.empty() && base.back() == '/') base.pop_back();
  char name[96];
  std::snprintf(name, sizeof(name), "/tpcdaq_test_ped_table_%d_%s.json",
                static_cast<int>(::getpid()), tag);
  const std::string path = base + name;
  FILE* fp = std::fopen(path.c_str(), "wb");
  if (fp != nullptr) {
    std::fwrite(text.data(), 1, text.size(), fp);
    std::fclose(fp);
  }
  return path;
}

// `calib pedestal` の pretty 出力と同じ形の 1 要素。
std::string channel_json(const char* asad, const char* aget, const char* ch, const char* mean) {
  return std::string("    {\n      \"asad\": ") + asad + ",\n      \"aget\": " + aget +
         ",\n      \"ch\": " + ch + ",\n      \"mean\": " + mean +
         ",\n      \"rms\": 6.5,\n      \"runs\": 1,\n      \"is_fpn\": false\n    }";
}

const char* kMeta =
    "  \"meta\": {\n"
    "    \"inputs\": [\n"
    "      \"/data/run0133/CoBo0_AsAd0_2026-10-15T10:00:00.000_0000.graw\"\n"
    "    ],\n"
    "    \"events\": 500,\n"
    "    \"generated_at\": \"2026-10-16T09:12:34.567+00:00\",\n"
    "    \"tool_version\": \"0.1.0\"\n"
    "  }";

// ---------------------------------------------------------------------------
// 1. 読めた値は (asad, aget, ch) のマスへ、無い ch は NaN
// ---------------------------------------------------------------------------
//
// 表の中身(非対称な値。同じ値を並べない):
//   (0,0,0)  = 251.375     ← 通常 ch
//   (0,0,11) = 300.5       ← FPN(raw 11)。表は FPN も持つ(使うかは呼び手次第)
//   (1,3,67) = 412.25      ← 各軸の上端
//   (3,2,5)  = 199         ← asad の上端(4 AsAd = 1 論理 CoBo の ELITPC)
//   (0,1,1)  = null        ← serde_json は NaN を null で書く → 使えない要素(skip + 数える)
//   (4,0,0) / (0,4,0) / (0,0,68) ← 枠の外(asad 0–3 / aget 0–3 / ch 0–67)→ skip + 数える
// → channels = 4、entries_skipped = 4(null 1 + 枠外 3)。
void test_load_reads_means_into_the_grid() {
  const std::string text = std::string("{\n") + kMeta + ",\n  \"channels\": [\n" +
                           channel_json("0", "0", "0", "251.375") + ",\n" +
                           channel_json("0", "0", "11", "300.5") + ",\n" +
                           channel_json("1", "3", "67", "412.25") + ",\n" +
                           channel_json("3", "2", "5", "199.0") + ",\n" +
                           channel_json("0", "1", "1", "null") + ",\n" +
                           channel_json("4", "0", "0", "500.0") + ",\n" +
                           channel_json("0", "4", "0", "501.0") + ",\n" +
                           channel_json("0", "0", "68", "502.0") + "\n  ]\n}\n";
  const std::string path = scratch_file("grid", text);

  tpcpevent::PedTable table;
  std::string err;
  CHECK(tpcpevent::load_ped_table(path, &table, &err));
  CHECK(err.empty());
  CHECK(table.path == path);
  CHECK(table.generated_at == "2026-10-16T09:12:34.567+00:00");
  CHECK_EQ(table.channels, 4);
  CHECK_EQ(table.entries_skipped, 4);

  CHECK_D(table.mean_of(0, 0, 0), 251.375);
  CHECK_D(table.mean_of(0, 0, 11), 300.5);
  CHECK_D(table.mean_of(1, 3, 67), 412.25);
  CHECK_D(table.mean_of(3, 2, 5), 199.0);
  // 表に無い ch は NaN(呼び手は「表に無い」として skip + 計数する)。
  CHECK(std::isnan(table.mean_of(0, 0, 1)));
  CHECK(std::isnan(table.mean_of(2, 0, 0)));
  // null は値として採らない。
  CHECK(std::isnan(table.mean_of(0, 1, 1)));
  // 引く側の枠外も NaN(配列をはみ出して読まない)。
  CHECK(std::isnan(table.mean_of(4, 0, 0)));
  CHECK(std::isnan(table.mean_of(0, 4, 0)));
  CHECK(std::isnan(table.mean_of(0, 0, 68)));
  CHECK(std::isnan(table.mean_of(255, 3, 67)));
  std::remove(path.c_str());
}

// 既定構築の表は全マス NaN(テストが直接マスに書いて使う形)。
void test_default_table_is_all_nan() {
  const tpcpevent::PedTable table;
  CHECK(std::isnan(table.mean_of(0, 0, 0)));
  CHECK(std::isnan(table.mean_of(3, 3, 67)));
  CHECK_EQ(table.channels, 0);
}

// ---------------------------------------------------------------------------
// 2. 起動エラーになる 3 通り(SPEC §3.2「半端な設定で走らない」)
// ---------------------------------------------------------------------------

void test_missing_file_is_an_error() {
  tpcpevent::PedTable table;
  std::string err;
  CHECK(!tpcpevent::load_ped_table("/nonexistent/tpcdaq/pedestal_502ns.json", &table, &err));
  CHECK(err.find("cannot open") != std::string::npos);
  CHECK(err.find("/nonexistent/tpcdaq/pedestal_502ns.json") != std::string::npos);
}

void test_no_channels_key_is_an_error() {
  const std::string path = scratch_file("nochannels", std::string("{\n") + kMeta + "\n}\n");
  tpcpevent::PedTable table;
  std::string err;
  CHECK(!tpcpevent::load_ped_table(path, &table, &err));
  CHECK(err.find("\"channels\"") != std::string::npos);
  std::remove(path.c_str());
}

// channels はあるが使える要素が 1 つも無い(空配列)—— 全 ZS フレームの全 ch が
// 「表に無い」になるだけの表で走らない(tools/scan/run_metrics.C loadPedTable と同じ判定)。
void test_channels_without_usable_entries_is_an_error() {
  const std::string path =
      scratch_file("empty", std::string("{\n") + kMeta + ",\n  \"channels\": []\n}\n");
  tpcpevent::PedTable table;
  std::string err;
  CHECK(!tpcpevent::load_ped_table(path, &table, &err));
  CHECK(err.find("no usable") != std::string::npos);
  std::remove(path.c_str());
}

}  // namespace

int main() {
  test_load_reads_means_into_the_grid();
  test_default_table_is_all_nan();
  test_missing_file_is_an_error();
  test_no_channels_key_is_an_error();
  test_channels_without_usable_entries_is_an_error();
  return tpccheck::report("test_ped_table");
}
