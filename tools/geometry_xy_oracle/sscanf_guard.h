// sscanf_guard.h — TPCReco GeometryTPC::LoadAnalog のスタック溢れを避けるビルド時の当て物
// (TODO/125 レーン A-4)。Makefile が `-include` で全 TU の先頭に差し込む。TPCReco のソースは
// 一切書き換えない。
//
// 何が起きるか: HIGS2026_online の GeometryTPC.cpp:506-508(LoadAnalog)は、要素数 12 の name と
// 要素数 1 の rest を書式 "%12s %d %d %d %d %s" の sscanf で .dat の**全行**から読む。6 欄以上の
// 行(strip 行は 10 欄)では最後の %s が rest に 2 バイト以上書き、%12s も name を 1 バイト
// 溢れうる(未定義動作。TPCReco の本番環境で表面化しているかは未確認)。この Mac(clang -O2)
// では Load の終わりで std::string の破棄が壊れたポインタを free して abort する(2026-10-13
// 実測。-fsanitize=address で stack-buffer-overflow @ GeometryTPC.cpp:508 を確認)。
//
// 当て物: この**書式文字列のときだけ**、幅を守った書式で読み直す。戻り値(5 = AUX 行 / 6 = それ
// 以外)は元と同じになる —— LoadAnalog が見るのは戻り値と name/cobo/asad/aget/chan_num だけで、
// rest の中身は使われない。他の書式はそのまま vsscanf へ渡す。

#ifndef TPCDAQ_GEOMETRY_XY_ORACLE_SSCANF_GUARD_H
#define TPCDAQ_GEOMETRY_XY_ORACLE_SSCANF_GUARD_H

#include <cstdarg>
#include <cstdio>
#include <cstring>

inline int tpcdaq_oracle_sscanf(const char *str, const char *format, ...) {
  va_list ap;
  va_start(ap, format);
  int result;
  if (std::strcmp(format, "%12s %d %d %d %d %s") == 0) {
    char *name = va_arg(ap, char *);
    int *cobo = va_arg(ap, int *);
    int *asad = va_arg(ap, int *);
    int *aget = va_arg(ap, int *);
    int *chan = va_arg(ap, int *);
    char rest[2];  // 呼び手の rest[1] には書かない
    result = std::sscanf(str, "%11s %d %d %d %d %1s", name, cobo, asad, aget, chan, rest);
  } else {
    result = std::vsscanf(str, format, ap);
  }
  va_end(ap);
  return result;
}

#define sscanf tpcdaq_oracle_sscanf

#endif
