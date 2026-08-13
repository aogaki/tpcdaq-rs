// compare_pevent.cxx — PEventTPC TTree の全値比較(TODO/020 §6、SPEC §12-3 v1.8 の
// 実データ照合に使う道具)。
//
//   compare_pevent <a.root> <b.root> [tree_a] [tree_b]
//
// **eventId で突き合わせる**(エントリ順に依存しない)。我々の出力は event_idx 昇順、
// grawToEventTPC の出力は .graw の到着順 —— 順序で偽陰性にしない。
// 比較するもの:
//   * eventId の集合(片方にしか無い eventId を列挙)
//   * EventInfo の各フィールド(runId / eventId / timestamp / eventType /
//     pedestalSubtracted / global_properties 3 つ)
//   * chargeMap の**全 key と値**(許容差 **0** —— 同じ算法なら 1 bit も違わない)
//
// 終了コード: 0 = 一致 / 1 = 差分あり / 2 = 使い方・IO エラー。
//
// **メモリ**: 実機ファイルは 1 エントリの chargeMap が 17 MB(11 GB / 3852 entries)。
// 全部載せずに「①EventInfo だけ読んで eventId → entry の索引を作る ②共通 eventId を
// 1 件ずつ両側から読んで比べる」の 2 パスにしてある。

#include <cstdarg>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <map>
#include <string>
#include <vector>

#include <TFile.h>
#include <TTree.h>

#include "TPCReco/PEventTPC.h"

namespace {

constexpr int kExitMatch = 0;
constexpr int kExitDiff = 1;
constexpr int kExitUsage = 2;
constexpr size_t kMaxReportedDiffs = 20;

void usage(const char* argv0) {
  std::printf(
      "usage: %s <a.root> <b.root> [tree_a] [tree_b]\n"
      "\n"
      "Compares two PEventTPC trees (default tree name: TPCData) event by event,\n"
      "matched on EventInfo::eventId. Every EventInfo field and every chargeMap\n"
      "key/value must match exactly (tolerance 0).\n"
      "\n"
      "exit: 0 = identical, 1 = differences, 2 = usage or IO error\n",
      argv0);
}

// 差分の台帳。先頭 kMaxReportedDiffs 件だけ出して、総数は必ず出す。
class DiffLog {
 public:
  void add(const std::string& msg) {
    ++count_;
    if (count_ <= kMaxReportedDiffs) std::fprintf(stderr, "DIFF %s\n", msg.c_str());
  }
  void addf(const char* fmt, ...) __attribute__((format(printf, 2, 3))) {
    char buf[512];
    va_list ap;
    va_start(ap, fmt);
    std::vsnprintf(buf, sizeof(buf), fmt, ap);
    va_end(ap);
    add(buf);
  }
  size_t count() const { return count_; }

 private:
  size_t count_ = 0;
};

struct Side {
  TFile* file = nullptr;
  TTree* tree = nullptr;
  PEventTPC* event = nullptr;
  std::map<unsigned, Long64_t> by_event_id;  // eventId → entry
  size_t duplicate_event_ids = 0;
};

bool open_side(const std::string& path, const std::string& tree_name, Side* s) {
  s->file = TFile::Open(path.c_str(), "READ");
  if (s->file == nullptr || s->file->IsZombie()) {
    std::fprintf(stderr, "compare_pevent: cannot open %s\n", path.c_str());
    return false;
  }
  s->tree = dynamic_cast<TTree*>(s->file->Get(tree_name.c_str()));
  if (s->tree == nullptr) {
    std::fprintf(stderr, "compare_pevent: no TTree named \"%s\" in %s\n", tree_name.c_str(),
                 path.c_str());
    return false;
  }
  s->tree->SetBranchAddress("Event", &s->event);
  return true;
}

// ①EventInfo だけ読んで eventId → entry の索引を作る(chargeMap は読まない)。
void index_events(Side* s) {
  s->tree->SetBranchStatus("myChargeMap*", false);
  for (Long64_t i = 0; i < s->tree->GetEntries(); ++i) {
    s->tree->GetEntry(i);
    const unsigned id = s->event->GetEventInfo().GetEventId();
    if (!s->by_event_id.insert({id, i}).second) ++s->duplicate_event_ids;
  }
  s->tree->SetBranchStatus("*", true);
}

void compare_event_info(unsigned id, const eventraw::EventInfo& a,
                        const eventraw::EventInfo& b, DiffLog* log) {
  if (a.GetRunId() != b.GetRunId()) {
    log->addf("event %u: runId A=%ld B=%ld", id, a.GetRunId(), b.GetRunId());
  }
  if (a.GetEventId() != b.GetEventId()) {
    log->addf("event %u: eventId A=%u B=%u", id, a.GetEventId(), b.GetEventId());
  }
  if (a.GetEventTimestamp() != b.GetEventTimestamp()) {
    log->addf("event %u: timestamp A=%lu B=%lu", id,
              static_cast<unsigned long>(a.GetEventTimestamp()),
              static_cast<unsigned long>(b.GetEventTimestamp()));
  }
  if (a.GetEventType() != b.GetEventType()) {
    log->addf("event %u: eventType A=%s B=%s", id, a.GetEventType().to_string().c_str(),
              b.GetEventType().to_string().c_str());
  }
  if (a.GetPedestalSubtracted() != b.GetPedestalSubtracted()) {
    log->addf("event %u: pedestalSubtracted A=%d B=%d", id,
              a.GetPedestalSubtracted() ? 1 : 0, b.GetPedestalSubtracted() ? 1 : 0);
  }
  const auto pa = a.GetProperties();
  const auto pb = b.GetProperties();
  if (pa.max_charge != pb.max_charge || pa.integrated_charge != pb.integrated_charge ||
      pa.n_hits != pb.n_hits) {
    log->addf("event %u: global_properties A=(%d,%d,%d) B=(%d,%d,%d)", id, pa.max_charge,
              pa.integrated_charge, pa.n_hits, pb.max_charge, pb.integrated_charge,
              pb.n_hits);
  }
}

void compare_charge_maps(unsigned id, const PEventTPC::chargeMapType& a,
                         const PEventTPC::chargeMapType& b, DiffLog* log) {
  if (a.size() != b.size()) {
    log->addf("event %u: chargeMap size A=%zu B=%zu", id, a.size(), b.size());
  }
  auto ia = a.begin();
  auto ib = b.begin();
  while (ia != a.end() && ib != b.end()) {
    if (ia->first < ib->first) {
      log->addf("event %u: key {%d,%d,%d,%d} only in A (value %.17g)", id,
                std::get<0>(ia->first), std::get<1>(ia->first), std::get<2>(ia->first),
                std::get<3>(ia->first), ia->second);
      ++ia;
    } else if (ib->first < ia->first) {
      log->addf("event %u: key {%d,%d,%d,%d} only in B (value %.17g)", id,
                std::get<0>(ib->first), std::get<1>(ib->first), std::get<2>(ib->first),
                std::get<3>(ib->first), ib->second);
      ++ib;
    } else {
      if (ia->second != ib->second) {  // 許容差 0
        log->addf("event %u: key {%d,%d,%d,%d} A=%.17g B=%.17g", id, std::get<0>(ia->first),
                  std::get<1>(ia->first), std::get<2>(ia->first), std::get<3>(ia->first),
                  ia->second, ib->second);
      }
      ++ia;
      ++ib;
    }
  }
  for (; ia != a.end(); ++ia) {
    log->addf("event %u: key {%d,%d,%d,%d} only in A (value %.17g)", id,
              std::get<0>(ia->first), std::get<1>(ia->first), std::get<2>(ia->first),
              std::get<3>(ia->first), ia->second);
  }
  for (; ib != b.end(); ++ib) {
    log->addf("event %u: key {%d,%d,%d,%d} only in B (value %.17g)", id,
              std::get<0>(ib->first), std::get<1>(ib->first), std::get<2>(ib->first),
              std::get<3>(ib->first), ib->second);
  }
}

}  // namespace

int main(int argc, char** argv) {
  std::vector<std::string> positional;
  for (int i = 1; i < argc; ++i) {
    const std::string arg = argv[i];
    if (arg == "-h" || arg == "--help") {
      usage(argv[0]);
      return kExitUsage;
    }
    positional.push_back(arg);
  }
  if (positional.size() < 2 || positional.size() > 4) {
    usage(argv[0]);
    return kExitUsage;
  }
  const std::string tree_a = positional.size() >= 3 ? positional[2] : "TPCData";
  const std::string tree_b = positional.size() >= 4 ? positional[3] : tree_a;

  Side a;
  Side b;
  if (!open_side(positional[0], tree_a, &a) || !open_side(positional[1], tree_b, &b)) {
    return kExitUsage;
  }
  index_events(&a);
  index_events(&b);

  std::printf("A: %s tree=%s entries=%lld unique_event_ids=%zu\n", positional[0].c_str(),
              tree_a.c_str(), a.tree->GetEntries(), a.by_event_id.size());
  std::printf("B: %s tree=%s entries=%lld unique_event_ids=%zu\n", positional[1].c_str(),
              tree_b.c_str(), b.tree->GetEntries(), b.by_event_id.size());

  DiffLog log;
  if (a.duplicate_event_ids != 0 || b.duplicate_event_ids != 0) {
    log.addf("duplicate eventIds: A=%zu B=%zu (cannot match one-to-one)",
             a.duplicate_event_ids, b.duplicate_event_ids);
  }
  for (const auto& [id, entry] : a.by_event_id) {
    (void)entry;
    if (b.by_event_id.count(id) == 0) log.addf("event %u: only in A", id);
  }
  for (const auto& [id, entry] : b.by_event_id) {
    (void)entry;
    if (a.by_event_id.count(id) == 0) log.addf("event %u: only in B", id);
  }

  size_t compared = 0;
  for (const auto& [id, entry_a] : a.by_event_id) {
    const auto it = b.by_event_id.find(id);
    if (it == b.by_event_id.end()) continue;
    a.tree->GetEntry(entry_a);
    // **A を読んだ直後にコピーする**: 次の GetEntry で A のバッファは上書きされる。
    const eventraw::EventInfo info_a = a.event->GetEventInfo();
    const PEventTPC::chargeMapType map_a = a.event->GetChargeMap();
    b.tree->GetEntry(it->second);
    ++compared;
    compare_event_info(id, info_a, b.event->GetEventInfo(), &log);
    compare_charge_maps(id, map_a, b.event->GetChargeMap(), &log);
  }

  std::printf("compared %zu events, %zu differences\n", compared, log.count());
  if (log.count() > kMaxReportedDiffs) {
    std::fprintf(stderr, "(%zu more differences not shown)\n",
                 log.count() - kMaxReportedDiffs);
  }
  a.file->Close();
  b.file->Close();
  return log.count() == 0 ? kExitMatch : kExitDiff;
}
