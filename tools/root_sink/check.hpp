// check.hpp — 最小のテストハーネス(delila-rs root_sink の試験流儀: 素の assert + main)。
//
// フレームワークを持ち込まない。CHECK は失敗しても走り続け、最後に件数を出して
// 非 0 で終わる(どのケースが落ちたか全部見える方が原因究明が速い)。

#ifndef TPCDAQ_ROOT_SINK_CHECK_HPP
#define TPCDAQ_ROOT_SINK_CHECK_HPP

#include <cstdio>

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
#define CHECK_EQ(actual, expected)                                             \
  do {                                                                         \
    const long long a_ = static_cast<long long>(actual);                       \
    const long long e_ = static_cast<long long>(expected);                     \
    if (a_ == e_) {                                                            \
      ++tpccheck::g_pass;                                                      \
    } else {                                                                   \
      ++tpccheck::g_fail;                                                      \
      std::printf("FAIL %s:%d  %s == %s (got %lld, want %lld)\n", __FILE__,    \
                  __LINE__, #actual, #expected, a_, e_);                       \
    }                                                                          \
  } while (0)

#endif  // TPCDAQ_ROOT_SINK_CHECK_HPP
