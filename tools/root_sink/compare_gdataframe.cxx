// compare_gdataframe.cxx — GDataFrame TTree の全値比較(TODO/012 §1、SPEC §12-3 の
// 「比較スクリプトを tools/ に置く」の実体)。
//
//   compare_gdataframe <a.root> <b.root> [tree_a] [tree_b]
//                      [--ignore-field NAME]... [--strict-order]
//
// **エントリ順に依存しない**のが要点。我々の出力は eventIdx 昇順 + イベント内 (cobo,asad)
// 昇順(SPEC §6.3 v1.3)、実機オラクルは .graw 到着順。チャンネルの並びも我々 = (aget,chan)
// 昇順 / 本家 graw2root = (chan,aget) 昇順で異なる(SPEC §6.4)。したがって
//
//   * エントリは **(eventIdx, coboIdx, asadIdx)** をキーに突き合わせる
//   * チャンネルは **(aget, chan)** をキーに突き合わせる
//   * チャンネル内のサンプルは (bucket, value) の列として順序込みで比較する
//     (SPEC §6.4「チャンネル内サンプルは連続で AddSample」= 標準リーダの前提)
//
// **暗黙のホワイトリストを作らない**(TODO/012 §1): 既知の許容差は `--ignore-field` で
// 呼び手が明示し、無視した項目は必ず出力に列挙する。
//
//   --ignore-field NAME  GFrameHeader のフィールドを比較から外す。NAME は
//                        fDataSource / fRevision / fEventTime / fReadOffset / fStatus /
//                        fMult / fWindowOut / fLastCellIdx / fHitPatterns のいずれか
//                        (fEventIdx / fCoboIdx / fAsadIdx はキーなので外せない)。
//   --strict-order       キー集合の一致に加えて **エントリの出現順**と
//                        **エントリ内のチャンネル出現順**も一致を要求する
//                        (決定性の検査 = SPEC v1.3 が効いていることの証明。TODO/012 §3)。
//
// 終了コード: 0 = 一致 / 1 = 差分あり / 2 = 使い方・IO エラー。
// 差分は先頭 20 件 + 総数を stderr へ、両側の要約を stdout へ 1 行ずつ出す。
//
// **GET クラスの地雷**(third_party/get は無改変): `GDataFrame` の TClonesArray は
// static 共有(fgChannels/fgSamples)で `~GDataFrame()` がそれを delete する。
// 2 個同時に生かしてはいけないので、**片方を読み切って素の C++ 構造体に落とし、
// TFile を閉じてから**もう片方を読む(test_recorder.cxx の同趣旨の注記を参照)。

#include <algorithm>
#include <cstdarg>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <map>
#include <set>
#include <string>
#include <utility>
#include <vector>

#include <TBits.h>
#include <TClonesArray.h>
#include <TFile.h>
#include <TKey.h>
#include <TList.h>
#include <TRefArray.h>
#include <TTree.h>

#include "GDataChannel.h"
#include "GDataFrame.h"
#include "GDataSample.h"
#include "GFrameHeader.h"

namespace {

constexpr int kExitEqual = 0;
constexpr int kExitDiff = 1;
constexpr int kExitUsage = 2;

constexpr size_t kMaxReportedDiffs = 20;

// ---------------------------------------------------------------------------
// 読み戻した中身(ROOT オブジェクトを一切持たない素の構造体)
// ---------------------------------------------------------------------------

struct Sample {
  unsigned short bucket = 0;
  unsigned short value = 0;
};

using ChanKey = std::pair<unsigned, unsigned>;  // (aget, chan)

struct Entry {
  unsigned data_source = 0;
  unsigned revision = 0;
  unsigned long long event_time = 0;
  unsigned read_offset = 0;
  unsigned status = 0;
  unsigned window_out = 0;
  unsigned mult[4] = {0, 0, 0, 0};
  unsigned last_cell[4] = {0, 0, 0, 0};
  std::string hit_patterns[4];  // '0'/'1' の並び(長さ = TBits::GetNbits())
  std::map<ChanKey, std::vector<Sample>> channels;
  std::vector<ChanKey> channel_order;  // ファイル内の出現順(--strict-order 用)
};

struct EntryKey {
  unsigned event_idx = 0;
  unsigned cobo = 0;
  unsigned asad = 0;
  bool operator<(const EntryKey& o) const {
    if (event_idx != o.event_idx) return event_idx < o.event_idx;
    if (cobo != o.cobo) return cobo < o.cobo;
    return asad < o.asad;
  }
  bool operator==(const EntryKey& o) const {
    return event_idx == o.event_idx && cobo == o.cobo && asad == o.asad;
  }
};

std::string to_string(const EntryKey& k) {
  char buf[96];
  std::snprintf(buf, sizeof(buf), "(eventIdx=%u, cobo=%u, asad=%u)", k.event_idx, k.cobo, k.asad);
  return buf;
}

struct FileData {
  std::string path;
  std::string tree_name;
  long long entries = 0;
  unsigned long long channels = 0;
  unsigned long long samples = 0;
  bool event_idx_nondecreasing = true;
  unsigned long long duplicate_keys = 0;
  std::map<unsigned, unsigned long long> cobo_entries;
  std::vector<EntryKey> order;
  std::map<EntryKey, Entry> by_key;
};

// ---------------------------------------------------------------------------
// TTree の解決 —— 指名 → 無ければ「ファイル内に唯一の TTree」へフォールバック
// ---------------------------------------------------------------------------

TTree* resolve_tree(TFile* file, const std::string& wanted, std::string* used) {
  TTree* tree = dynamic_cast<TTree*>(file->Get(wanted.c_str()));
  if (tree != nullptr) {
    *used = wanted;
    return tree;
  }
  // 名前で引けなかった: TTree のキー名を(サイクル違いを畳んで)集める。
  std::set<std::string> names;
  TList* keys = file->GetListOfKeys();
  if (keys != nullptr) {
    TIter it(keys);
    TObject* obj = nullptr;
    while ((obj = it()) != nullptr) {
      TKey* key = dynamic_cast<TKey*>(obj);
      if (key == nullptr) continue;
      if (std::strcmp(key->GetClassName(), "TTree") == 0) names.insert(key->GetName());
    }
  }
  if (names.size() != 1) {
    std::fprintf(stderr,
                 "compare_gdataframe: %s に \"%s\" が無く、代わりの唯一の TTree も決まらない"
                 "(候補 %zu 個)\n",
                 file->GetName(), wanted.c_str(), names.size());
    return nullptr;
  }
  *used = *names.begin();
  std::fprintf(stderr, "compare_gdataframe: %s に \"%s\" が無いので \"%s\" を使う\n",
               file->GetName(), wanted.c_str(), used->c_str());
  return dynamic_cast<TTree*>(file->Get(used->c_str()));
}

// ---------------------------------------------------------------------------
// 1 ファイルを読み切る(戻ってきた時点で GDataFrame は 1 個も生きていない)
// ---------------------------------------------------------------------------

bool read_file(const std::string& path, const std::string& wanted_tree, FileData* out) {
  out->path = path;
  TFile* file = TFile::Open(path.c_str(), "READ");
  if (file == nullptr || file->IsZombie()) {
    std::fprintf(stderr, "compare_gdataframe: %s を開けない\n", path.c_str());
    delete file;
    return false;
  }
  TTree* tree = resolve_tree(file, wanted_tree, &out->tree_name);
  if (tree == nullptr) {
    file->Close();
    delete file;
    return false;
  }

  GET::GDataFrame* frame = nullptr;
  if (tree->SetBranchAddress("GDataFrame", &frame) < 0) {
    std::fprintf(stderr, "compare_gdataframe: %s の \"%s\" に GDataFrame ブランチが無い\n",
                 path.c_str(), out->tree_name.c_str());
    file->Close();
    delete file;
    return false;
  }

  out->entries = tree->GetEntries();
  bool have_previous = false;
  unsigned previous_event_idx = 0;
  for (Long64_t i = 0; i < out->entries; ++i) {
    tree->GetEntry(i);
    const GET::GFrameHeader& h = frame->fHeader;
    EntryKey key;
    key.event_idx = h.fEventIdx;
    key.cobo = h.fCoboIdx;
    key.asad = h.fAsadIdx;

    if (have_previous && key.event_idx < previous_event_idx) out->event_idx_nondecreasing = false;
    previous_event_idx = key.event_idx;
    have_previous = true;

    out->order.push_back(key);
    out->cobo_entries[key.cobo] += 1;

    Entry entry;
    entry.data_source = h.fDataSource;
    entry.revision = h.fRevision;
    entry.event_time = static_cast<unsigned long long>(h.fEventTime);
    entry.read_offset = h.fReadOffset;
    entry.status = h.fStatus;
    entry.window_out = h.fWindowOut;
    for (int a = 0; a < 4; ++a) {
      entry.mult[a] = h.fMult[a];
      entry.last_cell[a] = h.fLastCellIdx[a];
      const TBits& bits = h.fHitPatterns[a];
      std::string pattern(bits.GetNbits(), '0');
      for (unsigned b = 0; b < bits.GetNbits(); ++b) {
        if (bits.TestBitNumber(b)) pattern[b] = '1';
      }
      entry.hit_patterns[a] = std::move(pattern);
    }

    TClonesArray* channels = frame->GetChannels();
    const int nchan = (channels == nullptr) ? 0 : static_cast<int>(channels->GetEntriesFast());
    for (int c = 0; c < nchan; ++c) {
      GET::GDataChannel* channel = static_cast<GET::GDataChannel*>(channels->At(c));
      if (channel == nullptr) continue;
      const ChanKey ck(channel->fAgetIdx, channel->fChanIdx);
      std::vector<Sample> samples;
      samples.reserve(channel->GetNhit());
      TRefArray& hits = channel->GetHits();
      for (int s = 0; s < channel->GetNhit(); ++s) {
        GET::GDataSample* sample = static_cast<GET::GDataSample*>(hits.At(s));
        if (sample == nullptr) continue;
        samples.push_back({sample->fBuckIdx, sample->fValue});
      }
      out->samples += samples.size();
      out->channels += 1;
      entry.channel_order.push_back(ck);
      if (!entry.channels.emplace(ck, std::move(samples)).second) {
        std::fprintf(stderr,
                     "compare_gdataframe: %s の %s に (aget=%u, chan=%u) が二度出てくる\n",
                     path.c_str(), to_string(key).c_str(), ck.first, ck.second);
      }
    }

    if (!out->by_key.emplace(key, std::move(entry)).second) {
      out->duplicate_keys += 1;
      std::fprintf(stderr, "compare_gdataframe: %s に重複キー %s\n", path.c_str(),
                   to_string(key).c_str());
    }
  }

  file->Close();
  delete file;  // TTree もろとも消える = GDataFrame は生き残らない
  return true;
}

// ---------------------------------------------------------------------------
// 差分の収集(先頭 kMaxReportedDiffs 件だけ文字列化、総数は全部数える)
// ---------------------------------------------------------------------------

class DiffLog {
 public:
  void add(const std::string& text) {
    ++total_;
    if (reported_.size() < kMaxReportedDiffs) reported_.push_back(text);
  }
  void addf(const char* format, ...) __attribute__((format(printf, 2, 3))) {
    ++total_;
    if (reported_.size() >= kMaxReportedDiffs) return;
    char buf[512];
    va_list args;
    va_start(args, format);
    std::vsnprintf(buf, sizeof(buf), format, args);
    va_end(args);
    reported_.push_back(buf);
  }
  unsigned long long total() const { return total_; }
  const std::vector<std::string>& reported() const { return reported_; }

 private:
  unsigned long long total_ = 0;
  std::vector<std::string> reported_;
};

// GFrameHeader のフィールド名(--ignore-field が受け付ける集合)。
const char* const kHeaderFields[] = {"fDataSource", "fRevision",    "fEventTime",  "fReadOffset",
                                     "fStatus",     "fMult",        "fWindowOut",  "fLastCellIdx",
                                     "fHitPatterns"};

bool is_header_field(const std::string& name) {
  for (const char* f : kHeaderFields) {
    if (name == f) return true;
  }
  return false;
}

void compare_header(const EntryKey& key, const Entry& a, const Entry& b,
                    const std::set<std::string>& ignored, DiffLog* log) {
  const std::string where = to_string(key);
  auto scalar = [&](const char* name, unsigned long long x, unsigned long long y) {
    if (ignored.count(name) != 0 || x == y) return;
    log->addf("%s %s: A=%llu B=%llu", where.c_str(), name, x, y);
  };
  scalar("fDataSource", a.data_source, b.data_source);
  scalar("fRevision", a.revision, b.revision);
  scalar("fEventTime", a.event_time, b.event_time);
  scalar("fReadOffset", a.read_offset, b.read_offset);
  scalar("fStatus", a.status, b.status);
  scalar("fWindowOut", a.window_out, b.window_out);
  if (ignored.count("fMult") == 0) {
    for (int i = 0; i < 4; ++i) {
      if (a.mult[i] != b.mult[i]) {
        log->addf("%s fMult[%d]: A=%u B=%u", where.c_str(), i, a.mult[i], b.mult[i]);
      }
    }
  }
  if (ignored.count("fLastCellIdx") == 0) {
    for (int i = 0; i < 4; ++i) {
      if (a.last_cell[i] != b.last_cell[i]) {
        log->addf("%s fLastCellIdx[%d]: A=%u B=%u", where.c_str(), i, a.last_cell[i],
                  b.last_cell[i]);
      }
    }
  }
  if (ignored.count("fHitPatterns") == 0) {
    for (int i = 0; i < 4; ++i) {
      if (a.hit_patterns[i] != b.hit_patterns[i]) {
        log->addf("%s fHitPatterns[%d]: A=\"%s\" B=\"%s\"", where.c_str(), i,
                  a.hit_patterns[i].c_str(), b.hit_patterns[i].c_str());
      }
    }
  }
}

void compare_channels(const EntryKey& key, const Entry& a, const Entry& b, bool strict_order,
                      DiffLog* log) {
  const std::string where = to_string(key);
  for (const auto& [ck, samples_a] : a.channels) {
    const auto it = b.channels.find(ck);
    if (it == b.channels.end()) {
      log->addf("%s channel (aget=%u, chan=%u): A のみ", where.c_str(), ck.first, ck.second);
      continue;
    }
    const std::vector<Sample>& samples_b = it->second;
    if (samples_a.size() != samples_b.size()) {
      log->addf("%s channel (aget=%u, chan=%u): サンプル数 A=%zu B=%zu", where.c_str(), ck.first,
                ck.second, samples_a.size(), samples_b.size());
      continue;
    }
    for (size_t s = 0; s < samples_a.size(); ++s) {
      if (samples_a[s].bucket != samples_b[s].bucket || samples_a[s].value != samples_b[s].value) {
        log->addf("%s (aget=%u, chan=%u) sample[%zu]: A=(bucket=%u, value=%u) "
                  "B=(bucket=%u, value=%u)",
                  where.c_str(), ck.first, ck.second, s, samples_a[s].bucket, samples_a[s].value,
                  samples_b[s].bucket, samples_b[s].value);
      }
    }
  }
  for (const auto& [ck, samples_b] : b.channels) {
    (void)samples_b;
    if (a.channels.find(ck) == a.channels.end()) {
      log->addf("%s channel (aget=%u, chan=%u): B のみ", where.c_str(), ck.first, ck.second);
    }
  }
  if (strict_order && a.channel_order != b.channel_order) {
    log->addf("%s: チャンネルの出現順が違う(--strict-order)", where.c_str());
  }
}

std::string cobo_entries_text(const std::map<unsigned, unsigned long long>& counts) {
  std::string out;
  for (const auto& [cobo, n] : counts) {
    if (!out.empty()) out += ",";
    char buf[64];
    std::snprintf(buf, sizeof(buf), "%u:%llu", cobo, n);
    out += buf;
  }
  return out.empty() ? std::string("-") : out;
}

void print_side(const char* tag, const FileData& d) {
  std::printf("%s: path=%s tree=%s entries=%lld keys=%zu channels=%llu samples=%llu "
              "event_idx_nondecreasing=%s duplicate_keys=%llu cobo_entries=%s\n",
              tag, d.path.c_str(), d.tree_name.c_str(), d.entries, d.by_key.size(), d.channels,
              d.samples, d.event_idx_nondecreasing ? "yes" : "no", d.duplicate_keys,
              cobo_entries_text(d.cobo_entries).c_str());
}

void usage(const char* argv0) {
  std::fprintf(stderr,
               "usage: %s <a.root> <b.root> [tree_a] [tree_b]\n"
               "           [--ignore-field NAME]... [--strict-order]\n"
               "\n"
               "  木名を省略すると \"tree\"、それが無ければファイル内で唯一の TTree。\n"
               "  --ignore-field NAME  GFrameHeader のフィールドを比較から外す\n"
               "                       (fDataSource / fRevision / fEventTime / fReadOffset /\n"
               "                        fStatus / fMult / fWindowOut / fLastCellIdx /\n"
               "                        fHitPatterns)\n"
               "  --strict-order       エントリとチャンネルの**出現順**の一致も要求する\n"
               "\n"
               "  exit 0 = 一致 / 1 = 差分あり / 2 = 使い方・IO エラー\n",
               argv0);
}

}  // namespace

int main(int argc, char** argv) {
  std::vector<std::string> positional;
  std::set<std::string> ignored;
  bool strict_order = false;

  for (int i = 1; i < argc; ++i) {
    const std::string arg = argv[i];
    if (arg == "-h" || arg == "--help") {
      usage(argv[0]);
      return kExitUsage;
    }
    if (arg == "--strict-order") {
      strict_order = true;
    } else if (arg == "--ignore-field" && i + 1 < argc) {
      const std::string name = argv[++i];
      if (!is_header_field(name)) {
        std::fprintf(stderr,
                     "compare_gdataframe: --ignore-field %s は GFrameHeader の比較対象名では"
                     "ない(fEventIdx / fCoboIdx / fAsadIdx はキーなので外せない)\n",
                     name.c_str());
        return kExitUsage;
      }
      ignored.insert(name);
    } else if (arg.rfind("--", 0) == 0) {
      std::fprintf(stderr, "compare_gdataframe: 不明な引数 '%s'\n", arg.c_str());
      usage(argv[0]);
      return kExitUsage;
    } else {
      positional.push_back(arg);
    }
  }
  if (positional.size() < 2 || positional.size() > 4) {
    usage(argv[0]);
    return kExitUsage;
  }
  const std::string tree_a = positional.size() >= 3 ? positional[2] : "tree";
  const std::string tree_b = positional.size() >= 4 ? positional[3] : tree_a;

  // **順に**読む(GDataFrame を 2 個同時に生かさない —— 先頭の注記)。
  FileData a;
  if (!read_file(positional[0], tree_a, &a)) return kExitUsage;
  FileData b;
  if (!read_file(positional[1], tree_b, &b)) return kExitUsage;

  print_side("A", a);
  print_side("B", b);

  DiffLog log;

  // 1. キー集合の一致
  for (const auto& [key, entry] : a.by_key) {
    (void)entry;
    if (b.by_key.find(key) == b.by_key.end()) log.add(to_string(key) + ": A のみ");
  }
  for (const auto& [key, entry] : b.by_key) {
    (void)entry;
    if (a.by_key.find(key) == a.by_key.end()) log.add(to_string(key) + ": B のみ");
  }
  if (a.duplicate_keys != 0 || b.duplicate_keys != 0) {
    log.addf("重複キー: A=%llu B=%llu(キーで突き合わせられない)", a.duplicate_keys,
             b.duplicate_keys);
  }

  // 2. 各エントリのヘッダとチャンネル
  unsigned long long compared_entries = 0;
  for (const auto& [key, entry_a] : a.by_key) {
    const auto it = b.by_key.find(key);
    if (it == b.by_key.end()) continue;
    ++compared_entries;
    compare_header(key, entry_a, it->second, ignored, &log);
    compare_channels(key, entry_a, it->second, strict_order, &log);
  }

  // 3. 出現順(--strict-order のときだけ)
  if (strict_order && a.order != b.order) {
    size_t first = 0;
    while (first < a.order.size() && first < b.order.size() && a.order[first] == b.order[first]) {
      ++first;
    }
    log.addf("エントリの出現順が違う(--strict-order): 最初の食い違いは entry %zu", first);
  }

  const char* ignored_text = "(なし)";
  std::string joined;
  if (!ignored.empty()) {
    for (const auto& name : ignored) {
      if (!joined.empty()) joined += ",";
      joined += name;
    }
    ignored_text = joined.c_str();
  }

  if (log.total() == 0) {
    std::printf("compare_gdataframe: OK — %llu エントリ / %llu チャンネル / %llu サンプルが一致"
                "(無視したフィールド: %s、strict_order=%s)\n",
                compared_entries, a.channels, a.samples, ignored_text,
                strict_order ? "yes" : "no");
    return kExitEqual;
  }

  std::fprintf(stderr, "compare_gdataframe: 差分 %llu 件(先頭 %zu 件):\n", log.total(),
               log.reported().size());
  for (const std::string& line : log.reported()) std::fprintf(stderr, "  %s\n", line.c_str());
  std::printf("compare_gdataframe: MISMATCH — 差分 %llu 件(無視したフィールド: %s、"
              "strict_order=%s)\n",
              log.total(), ignored_text, strict_order ? "yes" : "no");
  return kExitDiff;
}
