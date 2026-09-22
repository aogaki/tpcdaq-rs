# ccfg_resolve — CompoundConfig オラクル(078 レーン A)
#
# CompoundConfig / TinyXml のソースは **リポに入れない**(reference/ は .gitignore)。
# ビルド時に CCFG_DIR から参照するだけ。TPCReco と同じ流儀。
#
#   make -j                              # 既定の CCFG_DIR を使う
#   make -j CCFG_DIR=/path/to/CompoundConfig BOOST_INC=/opt/homebrew/include
#
# 注: CompoundConfig のヘッダは std::auto_ptr を使う(C++17 で削除された)ので
#     C++17 ではビルドできない。CXXSTD は c++11 を既定にしてある。

CCFG_DIR    ?= $(HOME)/Workspace/tpcdaq-rs/reference/20190315_patched/CompoundConfig
TINYXML_DIR ?= $(CCFG_DIR)/../TinyXml
BOOST_INC   ?= /opt/homebrew/include

CXX      ?= c++
CXXSTD   ?= -std=c++11
# CCfg/Io/BinaryFormat.h は autotools の config.h が定義する WORDS_*_ENDIAN を要求する
# (無いと #error)。autotools を回さないのでここで与える。x86_64 / arm64 = little。
ENDIAN_DEF ?= -DWORDS_LITTLE_ENDIAN
# CompoundConfig 側は tinyxml.h を include する前に TIXML_USE_STL を define する。
# TinyXml の .cpp を define 無しで積むと TiXmlElement のレイアウトが食い違って segfault する
# (2026-09-22、実測: XmlToConfigVisitor::isObject で SEGV)。全 TU で揃える。
CXXFLAGS ?= $(CXXSTD) -O2 -w $(ENDIAN_DEF) -DTIXML_USE_STL
INCLUDES  = -I$(CCFG_DIR)/src -I$(TINYXML_DIR)/src -I$(BOOST_INC)

BUILD ?= build
BIN    = $(BUILD)/ccfg_resolve

CCFG_SRCS = $(wildcard $(CCFG_DIR)/src/CCfg/*.cpp) \
            $(wildcard $(CCFG_DIR)/src/CCfg/Grammar/*.cpp) \
            $(wildcard $(CCFG_DIR)/src/CCfg/Io/*.cpp) \
            $(wildcard $(CCFG_DIR)/src/CCfg/View/*.cpp) \
            $(wildcard $(CCFG_DIR)/src/Utils/Math/*.cpp) \
            $(wildcard $(TINYXML_DIR)/src/*.cpp)

# 上記のファイル名は全て一意(確認済み)なので basename で平坦化してよい。
VPATH = $(sort $(dir $(CCFG_SRCS)))
OBJS  = $(addprefix $(BUILD)/,$(notdir $(CCFG_SRCS:.cpp=.o))) $(BUILD)/ccfg_resolve.o

all: $(BIN)

$(BIN): $(OBJS)
	$(CXX) $(CXXFLAGS) -o $@ $(OBJS)

$(BUILD)/ccfg_resolve.o: ccfg_resolve.cpp | $(BUILD)
	$(CXX) $(CXXFLAGS) $(INCLUDES) -c -o $@ $<

$(BUILD)/%.o: %.cpp | $(BUILD)
	$(CXX) $(CXXFLAGS) $(INCLUDES) -c -o $@ $<

$(BUILD):
	mkdir -p $(BUILD)

clean:
	rm -rf $(BUILD)

.PHONY: all clean
