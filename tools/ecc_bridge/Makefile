# tools/ecc_bridge/Makefile — fake-ECC(Ice servant のテストダブル)+ e2e クライアント
# (TODO/017、TODO/076 Phase 3、SPEC §8.2/§8.3)。
#
# ecc-bridge 本体は 076 Phase 3 で Rust 化・C++ 版は撤去済み(`cargo build --bin
# ecc_bridge` → `target/{debug,release}/ecc_bridge`)。ここに残るのは、Rust bin を
# 実 Ice サーバに擬した相手で叩くための C++ 一式(照合相手を変えないため)だけ:
#
#   make            fake_ecc / ecc_e2e_client をビルド(Ice + libzmq が要る)
#   make e2e        統合テスト run_ecc_e2e.sh(fake-ECC ↔ ecc_bridge(Rust bin)↔ ZMQ REQ)
#   make clean
#
# **Ice スタブは .ice から毎回生成し、コミットしない**(.gitignore)。`.ice` 自体も
# reference/ から**コピーしない** —— ビルド時にだけ参照する。参照パスは環境変数で渡す:
#
#   make TPCDAQ_ICE_DIR=$(cd ../.. && pwd)/reference/20190315_patched
#
# 未設定なら Ice を使うターゲットは**明示的に skip**(黙って劣化させない)。

CXX      ?= g++
CXXFLAGS ?= -std=c++17 -O2 -g -Wall -Wextra
LDFLAGS  ?=

ZMQ_CFLAGS := $(shell pkg-config --cflags libzmq 2>/dev/null)
ZMQ_LIBS   := $(shell pkg-config --libs libzmq 2>/dev/null)

# ZeroC Ice(Homebrew = 3.8.2)。ICE_HOME / -DICE_PREFIX で上書き可。
ICE_PREFIX ?= $(shell for d in "$$ICE_HOME" /opt/homebrew/opt/ice /usr/local/opt/ice /usr/local /usr; do \
                if [ -n "$$d" ] && [ -f "$$d/include/Ice/Ice.h" ]; then echo $$d; break; fi; done)
SLICE2CPP  ?= $(shell command -v slice2cpp 2>/dev/null || \
                (test -x "$(ICE_PREFIX)/bin/slice2cpp" && echo $(ICE_PREFIX)/bin/slice2cpp))

# GET / MDaq の .ice(実験使用中と同一版 = reference/20190315_patched)。
TPCDAQ_ICE_DIR ?=
MDAQ_SRC := $(TPCDAQ_ICE_DIR)/MDaq/src
GET_SRC  := $(TPCDAQ_ICE_DIR)/GetBench/src

# GetEcc.ice(get::rc::StateMachine)とその include 閉包。
# 流用元 ~/test/get/tpcdaq/CMakeLists.txt の _slices と同じ集合。
SLICE_UNITS := mdaq/Ecc mdaq/hw/Control mdaq/hw/HardwareTypes mdaq/utl/CmdException \
               get/GetEcc get/cobo/LedManager get/cobo/MutantLinkManager \
               get/mt/AlarmService get/mutant/AlignmentServer
SLICE_CPP   := $(addprefix slice/,$(addsuffix .cpp,$(SLICE_UNITS)))
SLICE_OBJ   := $(SLICE_CPP:.cpp=.o)

# Ice を使うターゲットを作れるか(3 条件が揃ったときだけ)。
ICE_OK := $(strip $(if $(TPCDAQ_ICE_DIR),$(if $(SLICE2CPP),$(if $(ICE_PREFIX),yes))))

ICE_CFLAGS := -I$(ICE_PREFIX)/include -Islice
ICE_LIBS   := -L$(ICE_PREFIX)/lib -lIce

# ecc_bridge.cpp / test_ecc_bridge.cpp は撤去したが、この 3 つは fake_ecc.cpp
# (ecc::State / ecc::next_state / ecc::parse_data_link_set = 共有状態機械)と
# ecc_e2e_client.cpp(check.hpp の CHECK マクロ、json_min.hpp の応答パース)が
# 今もそのまま使っている —— C++ 側に残る 2 バイナリの土台なので削除しない。
HEADERS := ecc_core.hpp json_min.hpp check.hpp

.PHONY: all e2e clean ice-skip

ifeq ($(ICE_OK),yes)
all: fake_ecc ecc_e2e_client
else
all: ice-skip
endif

ice-skip:
	@echo "ecc_bridge: SKIP (Ice ビルドの前提が揃っていない)"
	@echo "  TPCDAQ_ICE_DIR = '$(TPCDAQ_ICE_DIR)'  (.ice の場所。例 <repo>/reference/20190315_patched)"
	@echo "  slice2cpp      = '$(SLICE2CPP)'"
	@echo "  Ice prefix     = '$(ICE_PREFIX)'"
	@echo "  例: make TPCDAQ_ICE_DIR=\$$(cd ../.. && pwd)/reference/20190315_patched"

# --- Ice スタブ生成 --------------------------------------------------------
# 相互 include があるので **全部まとめて 1 回**生成し、スタンプで依存を張る。
slice/.stamp:
	@if [ ! -f "$(GET_SRC)/get/GetEcc.ice" ]; then \
	  echo "ERROR: $(GET_SRC)/get/GetEcc.ice が無い (TPCDAQ_ICE_DIR を確認)"; exit 1; fi
	@for u in $(SLICE_UNITS); do \
	  dir=slice/`dirname $$u`; mkdir -p $$dir; \
	  case $$u in mdaq/*) root=$(MDAQ_SRC);; *) root=$(GET_SRC);; esac; \
	  echo "slice2cpp $$u.ice"; \
	  $(SLICE2CPP) -I$(MDAQ_SRC) -I$(GET_SRC) --output-dir $$dir $$root/$$u.ice || exit 1; \
	done
	@touch $@

$(SLICE_CPP): slice/.stamp

# 生成コードは我々のコードではないので警告を抑制(-w)。
slice/%.o: slice/%.cpp
	$(CXX) $(CXXFLAGS) -w -I$(ICE_PREFIX)/include -Islice -c $< -o $@

# --- 実行ファイル ----------------------------------------------------------
fake_ecc: fake_ecc.cpp $(HEADERS) $(SLICE_OBJ)
	$(CXX) $(CXXFLAGS) $(ICE_CFLAGS) fake_ecc.cpp $(SLICE_OBJ) \
	  $(ICE_LIBS) $(LDFLAGS) -pthread -o $@

# e2e クライアントは Ice を要らない(ZMQ REQ + 生 TCP listen だけ)。
ecc_e2e_client: ecc_e2e_client.cpp $(HEADERS)
	$(CXX) $(CXXFLAGS) $(ZMQ_CFLAGS) ecc_e2e_client.cpp $(ZMQ_LIBS) $(LDFLAGS) -pthread -o $@

# --- テスト ----------------------------------------------------------------
ifeq ($(ICE_OK),yes)
e2e: all
	./run_ecc_e2e.sh
else
e2e: ice-skip
endif

clean:
	rm -f fake_ecc ecc_e2e_client
	rm -rf slice *.dSYM
