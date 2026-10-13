// event_mix — 実データの重ね合わせで複数飛跡イベントを作る(TODO/128)。
//   event_mix --in RAW.root --out MIX.root --random N [--seed S] [--max-shift D]
//   event_mix --in RAW.root --out MIX.root --pairs FILE
// 詳細は event_mix.hpp と README.md。
#include "event_mix.hpp"

static int usage() {
  std::cerr << "usage: event_mix --in RAW.root --out MIX.root (--random N [--seed S] [--max-shift D] | --pairs FILE)\n";
  return 2;
}

int main(int argc, char** argv) {
  std::string in, out, pairs;
  long nRandom = -1;
  unsigned long long seed = 1;
  int maxShift = 0;
  try {
    for (int i = 1; i < argc; ++i) {
      const std::string a = argv[i];
      if (i + 1 >= argc) return usage();
      const std::string v = argv[++i];
      if (a == "--in") in = v;
      else if (a == "--out") out = v;
      else if (a == "--pairs") pairs = v;
      else if (a == "--random") nRandom = std::stol(v);
      else if (a == "--seed") seed = std::stoull(v);
      else if (a == "--max-shift") maxShift = std::stoi(v);
      else return usage();
    }
    if (in.empty() || out.empty() || (pairs.empty() == (nRandom < 0))) return usage();
    int nIn = 0;
    {
      TFile f(in.c_str(), "READ");
      auto* t = f.IsZombie() ? nullptr : dynamic_cast<TTree*>(f.Get("raw"));
      if (!t) throw std::runtime_error("cannot read tree `raw` from " + in);
      nIn = static_cast<int>(t->GetEntries());
    }
    std::vector<evmix::MixSpec> specs;
    if (!pairs.empty()) {
      std::ifstream pf(pairs);
      if (!pf) throw std::runtime_error("cannot open " + pairs);
      specs = evmix::parseSpecs(pf, nIn);
    } else {
      specs = evmix::randomSpecs(nIn, static_cast<int>(nRandom), seed, maxShift);
    }
    const int n = evmix::mixFile(in, out, specs);
    std::cout << "event_mix: wrote " << n << " events to " << out << " (input " << nIn << " entries)\n";
  } catch (const std::exception& e) {
    std::cerr << "event_mix: ERROR: " << e.what() << "\n";
    return 1;
  }
  return 0;
}
