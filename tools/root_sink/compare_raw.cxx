// compare_raw.cxx — raw ADC ツリーの全値比較(TODO/075、SPEC §6.7 の受け入れ照合)。
//
//   compare_raw <a.root> <b.root>
//
// **オラクルは macros/graw2root.C の出力そのもの**(凍結)。同一 mini run について
// root-sink の `run{N}_raw.root` と graw2root.C の出力を、eventId / eventTime /
// adc 全要素で突き合わせる —— これ 1 本で機械判定できることが目的
// (compare_pevent と同じ役割)。
//
// **エントリ順で突き合わせる**(compare_pevent の eventId キーとは違う): raw ツリーは
// 「1 エントリ = 1 フレーム/イベントを読んだ順」であり、graw2root.C も root-sink も
// 同じ入力順(= event_idx 昇順)で書く。順序そのものが契約の一部なので、ここで
// 並べ替えて救済しない。
//
// 比較するもの:
//   * リーフの型名(eventId=UInt_t / eventTime=ULong64_t / adc=Short_t)と adc の長さ
//   * エントリ数(不一致なら両方の数を出し、共通部分だけ比較を続ける)
//   * 全エントリの eventId / eventTime / adc 全要素(許容差 0)
//
// 終了コード: 0 = 全一致 / 1 = 差分あり / 2 = open / tree 無し(使い方エラー含む)。

#include <cstdarg>
#include <cstdio>
#include <cstdlib>
#include <string>
#include <vector>

#include <TFile.h>
#include <TLeaf.h>
#include <TTree.h>

namespace {

constexpr int kExitMatch = 0;
constexpr int kExitDiff = 1;
constexpr int kExitOpen = 2;
constexpr std::size_t kMaxReportedDiffs = 10;

// ツリー名は契約(SPEC §6.7 / graw2root.C)で固定 —— 切り替えるノブは作らない。
constexpr const char* kTreeName = "raw";

void usage(const char* argv0) {
  std::printf(
      "usage: %s <a.root> <b.root>\n"
      "\n"
      "Compares two raw ADC trees (\"%s\") entry by entry: eventId, eventTime and\n"
      "every adc element must match exactly (tolerance 0).\n"
      "\n"
      "exit: 0 = identical, 1 = differences, 2 = open / no tree\n",
      argv0, kTreeName);
}

// 差分の台帳。先頭 kMaxReportedDiffs 件だけ出して、総数は必ず出す。
class DiffLog {
 public:
  void addf(const char* fmt, ...) __attribute__((format(printf, 2, 3))) {
    char buf[512];
    va_list ap;
    va_start(ap, fmt);
    std::vsnprintf(buf, sizeof(buf), fmt, ap);
    va_end(ap);
    ++count_;
    if (count_ <= kMaxReportedDiffs) std::fprintf(stderr, "DIFF %s\n", buf);
  }
  std::size_t count() const { return count_; }

 private:
  std::size_t count_ = 0;
};

struct Side {
  TFile* file = nullptr;
  TTree* tree = nullptr;
  UInt_t event_id = 0;
  ULong64_t event_time = 0;
  std::vector<Short_t> adc;
  std::string type_event_id;
  std::string type_event_time;
  std::string type_adc;
  Int_t adc_len = 0;
};

// リーフの型名(無ければ空文字 —— 呼び手が差分として報告する)。
std::string leaf_type(TTree* tree, const char* branch) {
  TLeaf* leaf = tree->GetLeaf(branch);
  return leaf == nullptr ? std::string() : std::string(leaf->GetTypeName());
}

bool open_side(const std::string& path, const std::string& tree_name, Side* s) {
  s->file = TFile::Open(path.c_str(), "READ");
  if (s->file == nullptr || s->file->IsZombie()) {
    std::fprintf(stderr, "compare_raw: cannot open %s\n", path.c_str());
    return false;
  }
  s->tree = dynamic_cast<TTree*>(s->file->Get(tree_name.c_str()));
  if (s->tree == nullptr) {
    std::fprintf(stderr, "compare_raw: no TTree named \"%s\" in %s\n", tree_name.c_str(),
                 path.c_str());
    return false;
  }
  s->type_event_id = leaf_type(s->tree, "eventId");
  s->type_event_time = leaf_type(s->tree, "eventTime");
  s->type_adc = leaf_type(s->tree, "adc");
  TLeaf* adc_leaf = s->tree->GetLeaf("adc");
  if (adc_leaf == nullptr) {
    std::fprintf(stderr, "compare_raw: %s has no \"adc\" branch\n", path.c_str());
    return false;
  }
  s->adc_len = adc_leaf->GetLenStatic();
  if (s->adc_len <= 0) {
    std::fprintf(stderr, "compare_raw: %s has adc length %d\n", path.c_str(), s->adc_len);
    return false;
  }
  s->adc.assign(static_cast<std::size_t>(s->adc_len), 0);
  s->tree->SetBranchAddress("eventId", &s->event_id);
  s->tree->SetBranchAddress("eventTime", &s->event_time);
  s->tree->SetBranchAddress("adc", s->adc.data());
  return true;
}

}  // namespace

int main(int argc, char** argv) {
  std::vector<std::string> positional;
  for (int i = 1; i < argc; ++i) {
    const std::string arg = argv[i];
    if (arg == "-h" || arg == "--help") {
      usage(argv[0]);
      return kExitOpen;
    }
    positional.push_back(arg);
  }
  if (positional.size() != 2) {
    usage(argv[0]);
    return kExitOpen;
  }
  const std::string tree_name = kTreeName;

  Side a;
  Side b;
  if (!open_side(positional[0], tree_name, &a) || !open_side(positional[1], tree_name, &b)) {
    return kExitOpen;
  }

  const Long64_t n_a = a.tree->GetEntries();
  const Long64_t n_b = b.tree->GetEntries();
  std::printf("A: %s tree=%s entries=%lld adc_len=%d\n", positional[0].c_str(),
              tree_name.c_str(), n_a, a.adc_len);
  std::printf("B: %s tree=%s entries=%lld adc_len=%d\n", positional[1].c_str(),
              tree_name.c_str(), n_b, b.adc_len);

  DiffLog log;
  if (a.type_event_id != b.type_event_id) {
    log.addf("leaf eventId type A=%s B=%s", a.type_event_id.c_str(),
             b.type_event_id.c_str());
  }
  if (a.type_event_time != b.type_event_time) {
    log.addf("leaf eventTime type A=%s B=%s", a.type_event_time.c_str(),
             b.type_event_time.c_str());
  }
  if (a.type_adc != b.type_adc) {
    log.addf("leaf adc type A=%s B=%s", a.type_adc.c_str(), b.type_adc.c_str());
  }
  if (n_a != n_b) log.addf("entries A=%lld B=%lld", n_a, n_b);
  // 長さが違うと要素比較の意味が無い(どの添字が対応するか決められない)。差分として
  // 報告し、比べられるもの(eventId / eventTime)だけ続ける —— 黙って飛ばさない。
  const bool compare_adc = (a.adc_len == b.adc_len);
  if (!compare_adc) {
    log.addf("adc length A=%d B=%d (element comparison skipped)", a.adc_len, b.adc_len);
  }

  const Long64_t common = (n_a < n_b) ? n_a : n_b;
  for (Long64_t i = 0; i < common; ++i) {
    a.tree->GetEntry(i);
    b.tree->GetEntry(i);
    if (a.event_id != b.event_id) {
      log.addf("entry %lld: eventId A=%u B=%u", i, a.event_id, b.event_id);
    }
    if (a.event_time != b.event_time) {
      log.addf("entry %lld: eventTime A=%llu B=%llu", i,
               static_cast<unsigned long long>(a.event_time),
               static_cast<unsigned long long>(b.event_time));
    }
    if (!compare_adc) continue;
    for (Int_t k = 0; k < a.adc_len; ++k) {
      const Short_t va = a.adc[static_cast<std::size_t>(k)];
      const Short_t vb = b.adc[static_cast<std::size_t>(k)];
      if (va != vb) {
        // 添字は生の通し番号と、契約の 3 次元 [aget-block][chan][bucket] の両方を出す。
        const Int_t bucket = k % 512;
        const Int_t chan = (k / 512) % 68;
        const Int_t block = k / (512 * 68);
        log.addf("entry %lld: adc[%d][%d][%d] (index %d) A=%d B=%d", i, block, chan, bucket,
                 k, static_cast<int>(va), static_cast<int>(vb));
      }
    }
  }

  std::printf("compared %lld events, %zu differences\n", common, log.count());
  if (log.count() > kMaxReportedDiffs) {
    std::fprintf(stderr, "(%zu more differences not shown)\n",
                 log.count() - kMaxReportedDiffs);
  }
  a.file->Close();
  b.file->Close();
  delete a.file;
  delete b.file;
  return log.count() == 0 ? kExitMatch : kExitDiff;
}
