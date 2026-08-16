// vcobo_link.hpp — データリンク(plain TCP)と graw リプレイ送出エンジン。**Ice を知らない**。
//
// docs/VIRTUAL_ZCOBO_ja.md §4.3 の「再現すべき挙動」11 項をそのまま実装する:
//   ① connect は configure 末尾に 1 回だけ(リトライを勝手に足さない)
//   ② configure 毎に旧接続を破棄してから新規 connect(破棄は connect 側の責務 — §4.5)
//   ③ daqStart / daqStop はソケットに触らない(stop→start を接続維持のまま繰り返せる)
//   ④ daqStop は flush のみ・打ち切らない(冪等)
//   ⑤ close は disconnect(breakup)か次の connect のみ・正常 FIN(shutdown+close)
//   ⑥ 送出はデータ駆動 + 3 s 端数 flush
//   ⑦ 背圧は完全ブロッキング(ドロップ機構を持たない)
//   ⑧ 送信エラー時に自動再接続しない(ログのみ)
//   ⑨ Nagle 有効(TCP_NODELAY を設定しない)・SO_KEEPALIVE 有効
//   ⑩ dataRouterType は大文字小文字を区別する完全一致
//   ⑪ plain TCP に topology フレームは無い(daqStart で追加送信しない)
#ifndef TPCDAQ_VCOBO_LINK_HPP
#define TPCDAQ_VCOBO_LINK_HPP

#include <arpa/inet.h>
#include <netinet/in.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#include <algorithm>
#include <atomic>
#include <cerrno>
#include <chrono>
#include <condition_variable>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <ctime>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

#include "vcobo_core.hpp"

namespace vcobo {

// ---------------------------------------------------------------------------
// ログ(ホットパスでは呼ばない —— フレーム毎の整形は禁止)
// ---------------------------------------------------------------------------
inline void log_line(const char* level, const std::string& msg) {
  std::time_t t = std::time(nullptr);
  char stamp[32] = {0};
  std::tm tm_buf;
  if (localtime_r(&t, &tm_buf) != nullptr) {
    std::strftime(stamp, sizeof(stamp), "%Y-%m-%d %H:%M:%S", &tm_buf);
  }
  std::fprintf(stdout, "%s %-5s %s\n", stamp, level, msg.c_str());
  std::fflush(stdout);
}
inline void log_info(const std::string& m) { log_line("INFO", m); }
inline void log_warn(const std::string& m) { log_line("WARN", m); }
inline void log_error(const std::string& m) { log_line("ERROR", m); }

// ---------------------------------------------------------------------------
// graw ファイル群 —— 起動時に全部読んでフレーム索引を作る。
// 送出中に heap 確保・open/close をしないための前処理(旧 DataBloc の失敗を繰り返さない)。
// ---------------------------------------------------------------------------
struct GrawSet {
  std::vector<uint8_t> bytes;      ///< 全ファイルの連結(**バイト無変換**)
  std::vector<FrameSpan> frames;   ///< 連結後の座標

  size_t total_bytes() const { return bytes.size(); }
};

/// ファイル群を読み込んでフレームに索引する。1 本でも読めなければ false。
///
/// TODO/056: 複数ファイル(ELITPC の AsAd 毎 4 本)は **eventIdx 昇順にマージした送出順**
/// に並べる。単純連結のままだと「AsAd0 全部 → AsAd1 全部 …」の順で流れ、イベントビルダが
/// 全イベントで残り 3 フラグメントを待って timeout する。並べ替えるのは**送出順だけ**で、
/// フレーム内のバイトには一切触らない。1 ファイルなら索引順のまま(mini 回帰は無変更)。
inline bool load_graw_set(const std::vector<std::string>& paths, GrawSet& set, std::string& err) {
  set.bytes.clear();
  set.frames.clear();

  // 4 × 1 GiB を読むので、vector の doubling による巨大な再確保を避けて先に確保する。
  // (見積もれなければ確保しないだけ —— 挙動は変わらない)
  size_t reserve_bytes = 0;
  for (const std::string& path : paths) {
    struct stat st;
    if (::stat(path.c_str(), &st) == 0 && st.st_size > 0) reserve_bytes += size_t(st.st_size);
  }
  if (reserve_bytes > 0) set.bytes.reserve(reserve_bytes);

  std::vector<std::vector<FrameSpan>> per_file;
  per_file.reserve(paths.size());

  for (const std::string& path : paths) {
    std::FILE* f = std::fopen(path.c_str(), "rb");
    if (f == nullptr) {
      err = "cannot open graw file '" + path + "'";
      return false;
    }
    const size_t base = set.bytes.size();
    uint8_t chunk[64 * 1024];
    size_t n = 0;
    while ((n = std::fread(chunk, 1, sizeof(chunk), f)) > 0) {
      set.bytes.insert(set.bytes.end(), chunk, chunk + n);
    }
    std::fclose(f);

    // ファイル単位で索引し、連結後の座標へ直す(ファイル境界を跨ぐフレームは無い)。
    std::vector<FrameSpan> spans;
    size_t trailing = 0;
    const bool clean = index_graw(set.bytes.data() + base, set.bytes.size() - base, spans, trailing);
    if (!clean) {
      // 端数は送らない(バイト一致を壊さないため)。黙って捨てず必ず可視化する。
      log_warn("graw file '" + path + "' ends with " + std::to_string(trailing) +
               " trailing bytes that do not form a complete frame; they will not be sent");
      set.bytes.resize(set.bytes.size() - trailing);
    }
    if (spans.empty()) {
      err = "graw file '" + path + "' contains no complete MFM frame";
      return false;
    }
    for (FrameSpan& s : spans) s.offset += base;
    log_info("graw '" + path + "': " + std::to_string(spans.size()) + " frames, " +
             std::to_string(set.bytes.size() - base) + " bytes");
    per_file.push_back(spans);
  }

  set.frames = merge_by_event_idx(set.bytes, per_file);

  // 送出順の素性を起動ログに出す(silent にしない)。
  uint32_t ev_min = 0;
  uint32_t ev_max = 0;
  size_t data_frames = 0;
  for (const FrameSpan& s : set.frames) {
    uint32_t ev = 0;
    if (!peek_event_idx(set.bytes.data() + s.offset, s.length, ev)) continue;
    if (data_frames == 0 || ev < ev_min) ev_min = ev;
    if (data_frames == 0 || ev > ev_max) ev_max = ev;
    ++data_frames;
  }
  const std::string range =
      data_frames == 0 ? std::string("none (no data frame)")
                       : std::to_string(ev_min) + ".." + std::to_string(ev_max);
  log_info("graw set: " + std::to_string(paths.size()) + " file(s), " +
           std::to_string(set.frames.size()) + " frames (" + std::to_string(data_frames) +
           " data), " + std::to_string(set.total_bytes()) + " bytes, eventIdx " + range +
           (paths.size() > 1 ? " (merged by eventIdx)" : ""));
  return true;
}

// ---------------------------------------------------------------------------
// データリンク + 送出エンジン
// ---------------------------------------------------------------------------
class DataLink {
 public:
  DataLink() = default;
  ~DataLink() { shutdown_worker(); }

  DataLink(const DataLink&) = delete;
  DataLink& operator=(const DataLink&) = delete;

  /// 起動時に 1 回。graw と設定を渡してワーカースレッドを立てる。
  void start(const GrawSet* set, const Config& cfg) {
    set_ = set;
    rate_hz_ = cfg.rate_hz;
    loop_ = cfg.loop;
    flush_bytes_ = cfg.flush_bytes;
    heartbeat_ = std::chrono::milliseconds(cfg.heartbeat_flush_ms);
    // 1 回の flush 分 + 最大フレーム 1 個が入る容量を先に確保する(以後 heap を触らない)。
    size_t max_frame = 0;
    for (const FrameSpan& s : set_->frames) max_frame = std::max(max_frame, s.length);
    pending_.reserve(flush_bytes_ + max_frame + 1);
    worker_ = std::thread(&DataLink::worker, this);
  }

  void shutdown_worker() {
    {
      std::lock_guard<std::mutex> lk(m_);
      quit_ = true;
      running_ = false;
    }
    cv_.notify_all();
    if (worker_.joinable()) worker_.join();
    close_socket();
  }

  // --- DaqCtrlNode の 5 op ------------------------------------------------

  /// `connect(dataRouterType, "ip:port")`。type は **"TCP" 完全一致**のみ受理。
  /// 既存センダがあれば自分で破棄してから、ブロッキング・単発・リトライ無しで接続する。
  bool connect(const std::string& type, const std::string& endpoint, std::string& err) {
    if (type != "TCP") {
      err = "unsupported dataRouterType '" + type + "' (this virtual CoBo only speaks \"TCP\")";
      return false;
    }
    std::string host;
    int port = 0;
    if (!split_endpoint(endpoint, host, port)) {
      err = "malformed dataRouterName '" + endpoint + "' (expected \"ip:port\")";
      return false;
    }

    // ②旧センダは connect 側が破棄する(ECC は再 configure で disconnect を送らない)。
    stop_sending();
    if (fd_ >= 0) {
      log_info("connect: destroying the previous data sender first");
      close_socket();
    }

    const int fd = ::socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) {
      err = std::string("socket() failed: ") + std::strerror(errno);
      return false;
    }
    int on = 1;
    // ⑨KEEPALIVE 有効。TCP_NODELAY は **設定しない**(Nagle 有効が実機)。
    ::setsockopt(fd, SOL_SOCKET, SO_KEEPALIVE, &on, sizeof(on));

    sockaddr_in addr;
    std::memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port = htons(uint16_t(port));
    if (::inet_pton(AF_INET, host.c_str(), &addr.sin_addr) != 1) {
      ::close(fd);
      err = "cannot resolve '" + host + "' as an IPv4 address";
      return false;
    }
    // ①ブロッキング・単発・リトライ無し。失敗は例外で ECC へ返す。
    if (::connect(fd, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) != 0) {
      const std::string reason = std::strerror(errno);
      ::close(fd);
      err = "connect to " + endpoint + " failed: " + reason;
      return false;
    }

    {
      std::lock_guard<std::mutex> lk(m_);
      fd_ = fd;
      send_failed_ = false;
    }
    log_info("data link established to " + endpoint + " (TCP, KEEPALIVE on, Nagle on)");
    return true;
  }

  /// ⑤breakup の disconnect。shutdown(both) + close で正常 FIN を出す。
  void disconnect() {
    stop_sending();
    if (fd_ < 0) {
      log_info("disconnect: no data link to close");
      return;
    }
    log_info("disconnect: shutdown(both) + close");
    close_socket();
  }

  /// ③ソケットには触らない。送出を先頭から開始する。
  bool daq_start(std::string& err) {
    if (fd_ < 0) {
      err = "daqStart without a data link (connect must come first)";
      return false;
    }
    {
      std::lock_guard<std::mutex> lk(m_);
      if (running_) return true;  // 冪等
      cursor_ = 0;                // run 毎に同じデータを流す(バイト一致の基準)
      running_ = true;
      stopped_ = false;
      restart_pacing_ = true;
    }
    cv_.notify_all();
    log_info("daqStart: replaying " + std::to_string(set_->frames.size()) + " frames");
    return true;
  }

  /// ④送出停止 + flush のみ。**冪等**(breakup で 2 度目が来る)。ソケットは開いたまま。
  void daq_stop() {
    bool was_running = false;
    {
      std::lock_guard<std::mutex> lk(m_);
      was_running = running_;
      running_ = false;
    }
    cv_.notify_all();
    if (!was_running) {
      log_info("daqStop: already stopped (idempotent)");
      return;
    }
    // 実機は最大 10 s 待って flush する(DaqCtrlNodeI.cpp:419-435)。
    std::unique_lock<std::mutex> lk(m_);
    if (!cv_.wait_for(lk, std::chrono::seconds(10), [this] { return stopped_; })) {
      log_warn("daqStop: sender did not settle within 10 s (still blocked on a write?)");
      return;
    }
    lk.unlock();
    log_info("daqStop: sent " + std::to_string(sent_bytes_.load()) + " bytes so far");
  }

  void set_always_flush(bool enable) { always_flush_ = enable; }

  void set_asad_mask(int mask) {
    std::lock_guard<std::mutex> lk(m_);
    asad_mask_ = mask;
  }

  int asad_mask() const {
    std::lock_guard<std::mutex> lk(m_);
    return asad_mask_;
  }

  bool linked() const { return fd_ >= 0; }
  uint64_t sent_bytes() const { return sent_bytes_.load(); }

 private:
  static bool split_endpoint(const std::string& s, std::string& host, int& port) {
    const size_t colon = s.rfind(':');
    if (colon == std::string::npos || colon == 0 || colon + 1 >= s.size()) return false;
    host = s.substr(0, colon);
    const std::string p = s.substr(colon + 1);
    for (char c : p) {
      if (c < '0' || c > '9') return false;
    }
    port = std::atoi(p.c_str());
    return port > 0 && port < 65536;
  }

  void stop_sending() {
    {
      std::lock_guard<std::mutex> lk(m_);
      if (!running_) return;
      running_ = false;
    }
    cv_.notify_all();
    std::unique_lock<std::mutex> lk(m_);
    cv_.wait_for(lk, std::chrono::seconds(10), [this] { return stopped_; });
  }

  void close_socket() {
    int fd = -1;
    {
      std::lock_guard<std::mutex> lk(m_);
      fd = fd_;
      fd_ = -1;
    }
    if (fd >= 0) {
      ::shutdown(fd, SHUT_RDWR);
      ::close(fd);
    }
  }

  /// ⑦背圧は完全ブロッキング。部分書き込みは書き切るまで回す。ドロップしない。
  bool send_all(const uint8_t* data, size_t len) {
    size_t off = 0;
    while (off < len) {
      const int fd = fd_;
      if (fd < 0) return false;
      const ssize_t n = ::send(fd, data + off, len - off, 0);
      if (n > 0) {
        off += size_t(n);
        continue;
      }
      if (n < 0 && errno == EINTR) continue;
      return false;
    }
    sent_bytes_ += len;
    return true;
  }

  void flush_pending() {
    if (pending_.empty()) {
      last_flush_ = std::chrono::steady_clock::now();
      return;
    }
    if (!send_all(pending_.data(), pending_.size())) {
      if (!send_failed_.exchange(true)) {
        // ⑧自動再接続しない。ログだけ出して静かに止まる(実機 MemRead.cpp:194-215)。
        log_error("send failed after " + std::to_string(sent_bytes_.load()) +
                  " bytes; not reconnecting (the real CoBo does not either)");
        std::lock_guard<std::mutex> lk(m_);
        running_ = false;
      }
    }
    pending_.clear();
    last_flush_ = std::chrono::steady_clock::now();
  }

  void maybe_flush(bool force) {
    const auto now = std::chrono::steady_clock::now();
    // ⑥3 s 端数 flush: 新しいフレームが無くてもハートビートで吐き出す。
    if (force || always_flush_ || pending_.size() >= flush_bytes_ ||
        (!pending_.empty() && now - last_flush_ >= heartbeat_)) {
      flush_pending();
    }
  }

  void worker() {
    last_flush_ = std::chrono::steady_clock::now();
    auto next_frame = std::chrono::steady_clock::now();

    while (true) {
      FrameSpan span;
      bool have_frame = false;
      {
        std::unique_lock<std::mutex> lk(m_);
        if (quit_) break;
        if (!running_) {
          if (!stopped_) {
            lk.unlock();
            maybe_flush(true);  // ④stop は flush して止まる
            lk.lock();
            stopped_ = true;
            cv_.notify_all();
          }
          cv_.wait_for(lk, std::chrono::milliseconds(50));
          continue;
        }
        if (restart_pacing_) {
          restart_pacing_ = false;
          next_frame = std::chrono::steady_clock::now();
        }
        if (cursor_ < set_->frames.size()) {
          span = set_->frames[cursor_++];
          have_frame = true;
        } else if (loop_) {
          cursor_ = 0;
          continue;
        }
      }

      if (!have_frame) {
        // 送り切った: リンクは保持したまま送出静止。端数は 3 s 後に吐く。
        maybe_flush(false);
        std::unique_lock<std::mutex> lk(m_);
        cv_.wait_for(lk, std::chrono::milliseconds(50));
        continue;
      }

      // ホットパス: memcpy のみ。heap 確保もログ整形もしない。
      const uint8_t* src = set_->bytes.data() + span.offset;
      pending_.insert(pending_.end(), src, src + span.length);
      maybe_flush(false);
      pace(next_frame);
    }
    maybe_flush(true);
  }

  void pace(std::chrono::steady_clock::time_point& next_frame) {
    if (rate_hz_ <= 0.0) return;  // 全速(031 負荷ハーネスモード)
    const auto period = std::chrono::nanoseconds(int64_t(1e9 / rate_hz_));
    next_frame += period;
    const auto now = std::chrono::steady_clock::now();
    if (next_frame <= now) {
      next_frame = now;  // 遅れは繰り越さない
      return;
    }
    std::this_thread::sleep_until(next_frame);
  }

  const GrawSet* set_ = nullptr;
  double rate_hz_ = 100.0;
  bool loop_ = false;
  size_t flush_bytes_ = 8192;
  std::chrono::milliseconds heartbeat_{3000};

  mutable std::mutex m_;
  std::condition_variable cv_;
  std::thread worker_;
  bool quit_ = false;
  bool running_ = false;
  bool stopped_ = true;
  bool restart_pacing_ = false;
  int asad_mask_ = 0;
  size_t cursor_ = 0;

  std::atomic<bool> always_flush_{false};
  std::atomic<int> fd_{-1};
  std::atomic<uint64_t> sent_bytes_{0};
  std::atomic<bool> send_failed_{false};

  // ワーカー専有(mutex の外)
  std::vector<uint8_t> pending_;
  std::chrono::steady_clock::time_point last_flush_;
};

}  // namespace vcobo

#endif  // TPCDAQ_VCOBO_LINK_HPP
