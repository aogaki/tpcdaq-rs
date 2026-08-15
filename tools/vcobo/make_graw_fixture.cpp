// make_graw_fixture — CI 用の**合成** .graw を書く(実データはリポに入れない)。
//
//   ./make_graw_fixture OUT.graw NFRAMES [BYTES_PER_FRAME]
//
// frameType 2 相当の符号化(metaType 0x08 = big-endian / blkSize 256)で、
// フレーム長は blkSize の倍数。中身は**フレーム毎に違うパターン**(非対称)にして、
// 順序の入れ替わりがバイト一致テストを素通りしないようにする。
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <string>
#include <vector>

#include "vcobo_core.hpp"

int main(int argc, char* argv[]) {
  if (argc < 3) {
    std::fprintf(stderr, "usage: make_graw_fixture OUT.graw NFRAMES [BYTES_PER_FRAME]\n");
    return 2;
  }
  const std::string out_path = argv[1];
  const long n_frames = std::atol(argv[2]);
  const long frame_bytes = (argc > 3) ? std::atol(argv[3]) : 512;

  if (n_frames <= 0 || frame_bytes < 256 || frame_bytes % 256 != 0) {
    std::fprintf(stderr, "NFRAMES must be > 0 and BYTES_PER_FRAME a positive multiple of 256\n");
    return 2;
  }

  std::FILE* f = std::fopen(out_path.c_str(), "wb");
  if (f == nullptr) {
    std::fprintf(stderr, "cannot write '%s'\n", out_path.c_str());
    return 1;
  }

  std::vector<uint8_t> frame(static_cast<size_t>(frame_bytes), 0u);
  const uint32_t blocks = uint32_t(frame_bytes / 256);
  for (long i = 0; i < n_frames; ++i) {
    frame[0] = 0x08;  // bit7=0 → big-endian、下位ニブル 8 → blkSize = 256
    frame[1] = uint8_t((blocks >> 16) & 0xFF);
    frame[2] = uint8_t((blocks >> 8) & 0xFF);
    frame[3] = uint8_t(blocks & 0xFF);
    frame[4] = 0x02;  // frameType 2(compact rev 5)
    frame[5] = 0x05;
    frame[6] = uint8_t((i >> 8) & 0xFF);  // 連番: 並べ替わったら一致しない
    frame[7] = uint8_t(i & 0xFF);
    for (size_t b = 8; b < frame.size(); ++b) {
      frame[b] = uint8_t((i * 31 + long(b) * 7) & 0xFF);
    }
    if (std::fwrite(frame.data(), 1, frame.size(), f) != frame.size()) {
      std::fclose(f);
      std::fprintf(stderr, "short write to '%s'\n", out_path.c_str());
      return 1;
    }
  }
  std::fclose(f);

  // 自分で書いたものが自分のフレーマで読めることを確認する(黙って壊れたものを出さない)。
  const size_t parsed = vcobo::graw_frame_size(frame.data());
  if (parsed != size_t(frame_bytes)) {
    std::fprintf(stderr, "internal error: header says %zu bytes, wanted %ld\n", parsed,
                 frame_bytes);
    return 1;
  }
  std::printf("%s: %ld frames x %ld bytes = %ld bytes\n", out_path.c_str(), n_frames, frame_bytes,
              n_frames * frame_bytes);
  return 0;
}
