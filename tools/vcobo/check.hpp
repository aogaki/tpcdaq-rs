// check.hpp — 最小のテストハーネス(素の assert + main)。
//
// **出自**: tools/ecc_bridge/check.hpp の逐語コピー(元は tools/root_sink/check.hpp)。
// vcobo は ecc_bridge / root_sink と別所有・別ビルドなので、ディレクトリを跨いで
// include せずコピーで独立させている(TODO/040「tools/vcobo/ 内で完結」)。
//
// フレームワークを持ち込まない。CHECK は失敗しても走り続け、最後に件数を出して
// 非 0 で終わる(どのケースが落ちたか全部見える方が原因究明が速い)。

#ifndef TPCDAQ_VCOBO_CHECK_HPP
#define TPCDAQ_VCOBO_CHECK_HPP

#include <cstdio>
#include <string>

namespace tpccheck {

inline int g_pass = 0;
inline int g_fail = 0;

// 全 CHECK の結果を出して終了コードを返す。main の末尾で `return tpccheck::report("name");`
inline int report(const char* suite) {
  std::printf("%s: %d passed, %d failed\n", suite, g_pass, g_fail);
  return g_fail == 0 ? 0 : 1;
}

}  // namespace tpccheck

#define CHECK(cond)                                                      \
  do {                                                                   \
    if (cond) {                                                          \
      ++tpccheck::g_pass;                                                \
    } else {                                                             \
      ++tpccheck::g_fail;                                                \
      std::printf("FAIL %s:%d  CHECK(%s)\n", __FILE__, __LINE__, #cond); \
    }                                                                    \
  } while (0)

// 期待値付き整数比較(落ちたときに実値が出る)。
#define CHECK_EQ(actual, expected)                                          \
  do {                                                                      \
    const long long a_ = static_cast<long long>(actual);                    \
    const long long e_ = static_cast<long long>(expected);                  \
    if (a_ == e_) {                                                         \
      ++tpccheck::g_pass;                                                   \
    } else {                                                                \
      ++tpccheck::g_fail;                                                   \
      std::printf("FAIL %s:%d  %s == %s (got %lld, want %lld)\n", __FILE__, \
                  __LINE__, #actual, #expected, a_, e_);                    \
    }                                                                       \
  } while (0)

// 文字列比較(017 追加)。DataLinkSet XML / JSON の**全文照合**が主用途なので、
// 落ちたときに両方の全文が出ること自体が仕様書になる。
#define CHECK_STR(actual, expected)                                        \
  do {                                                                     \
    const std::string a_ = (actual);                                       \
    const std::string e_ = (expected);                                     \
    if (a_ == e_) {                                                        \
      ++tpccheck::g_pass;                                                  \
    } else {                                                               \
      ++tpccheck::g_fail;                                                  \
      std::printf("FAIL %s:%d  %s\n  got  [%s]\n  want [%s]\n", __FILE__,  \
                  __LINE__, #actual, a_.c_str(), e_.c_str());              \
    }                                                                      \
  } while (0)

#endif  // TPCDAQ_VCOBO_CHECK_HPP
