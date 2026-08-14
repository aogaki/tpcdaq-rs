// json_min.hpp — ecc-bridge が話す JSON(SPEC §8.2)のための最小パーサ + エスケープ。
//
// **なぜ自前か**: ecc-bridge の依存は Ice + libzmq だけに閉じる(TODO/017 §1)。
// リクエストは `links` = オブジェクトの配列を含むので、command_router.cpp 流の
// 「部分文字列を拾う」やり方では入れ子で壊れる —— 退屈だが正しい再帰下降で書く。
//
// 方針: **never throw**(壊れた入力は false + 理由)、深さ上限あり、C++17、ヘッダのみ。

#ifndef TPCDAQ_ECC_BRIDGE_JSON_MIN_HPP
#define TPCDAQ_ECC_BRIDGE_JSON_MIN_HPP

#include <cstdio>
#include <cstdlib>
#include <string>
#include <utility>
#include <vector>

namespace jsonmin {

struct Value {
  enum class Type { Null, Bool, Number, String, Array, Object };

  Type type = Type::Null;
  bool boolean = false;
  double number = 0.0;
  std::string str;
  std::vector<Value> arr;                            // Array
  std::vector<std::pair<std::string, Value>> obj;    // Object(出現順を保つ)

  // Object のキー検索。無ければ nullptr(呼び手が既定値を決める)。
  const Value* find(const std::string& key) const {
    if (type != Type::Object) return nullptr;
    for (const auto& kv : obj) {
      if (kv.first == key) return &kv.second;
    }
    return nullptr;
  }
  std::string as_string() const { return type == Type::String ? str : std::string(); }
  long long as_int() const {
    return type == Type::Number ? static_cast<long long>(number) : 0;
  }
};

// JSON 文字列のエスケープ(制御文字は \u00XX)。
inline std::string escape(const std::string& s) {
  std::string out;
  out.reserve(s.size() + 8);
  for (const char ch : s) {
    const unsigned char c = static_cast<unsigned char>(ch);
    switch (c) {
      case '"': out += "\\\""; break;
      case '\\': out += "\\\\"; break;
      case '\n': out += "\\n"; break;
      case '\r': out += "\\r"; break;
      case '\t': out += "\\t"; break;
      case '\b': out += "\\b"; break;
      case '\f': out += "\\f"; break;
      default:
        if (c < 0x20) {
          char buf[8];
          std::snprintf(buf, sizeof(buf), "\\u%04x", c);
          out += buf;
        } else {
          out += ch;
        }
    }
  }
  return out;
}

namespace detail {

constexpr int kMaxDepth = 32;  // 敵対的な入れ子で stack を焼かない

struct Parser {
  const std::string& s;
  std::size_t i = 0;
  std::string err;

  explicit Parser(const std::string& text) : s(text) {}

  bool fail(const std::string& why) {
    if (err.empty()) err = "parse error at offset " + std::to_string(i) + ": " + why;
    return false;
  }
  void skip_ws() {
    while (i < s.size() && (s[i] == ' ' || s[i] == '\t' || s[i] == '\n' || s[i] == '\r')) ++i;
  }
  bool literal(const char* word) {
    const std::size_t n = std::char_traits<char>::length(word);
    if (s.compare(i, n, word) != 0) return fail(std::string("expected ") + word);
    i += n;
    return true;
  }

  bool parse_string(std::string& out) {
    if (i >= s.size() || s[i] != '"') return fail("expected string");
    ++i;
    out.clear();
    while (i < s.size()) {
      const char c = s[i++];
      if (c == '"') return true;
      if (c != '\\') {
        out += c;
        continue;
      }
      if (i >= s.size()) return fail("truncated escape");
      const char e = s[i++];
      switch (e) {
        case '"': out += '"'; break;
        case '\\': out += '\\'; break;
        case '/': out += '/'; break;
        case 'n': out += '\n'; break;
        case 'r': out += '\r'; break;
        case 't': out += '\t'; break;
        case 'b': out += '\b'; break;
        case 'f': out += '\f'; break;
        case 'u': {
          if (i + 4 > s.size()) return fail("truncated \\u escape");
          unsigned code = 0;
          for (int k = 0; k < 4; ++k) {
            const char h = s[i + static_cast<std::size_t>(k)];
            const unsigned d = (h >= '0' && h <= '9')   ? static_cast<unsigned>(h - '0')
                               : (h >= 'a' && h <= 'f') ? static_cast<unsigned>(h - 'a' + 10)
                               : (h >= 'A' && h <= 'F') ? static_cast<unsigned>(h - 'A' + 10)
                                                        : 16u;
            if (d == 16u) return fail("bad hex in \\u escape");
            code = code * 16 + d;
          }
          i += 4;
          // 我々が受ける値(action / id / IP)は ASCII。BMP を UTF-8 に落とすだけの最小実装。
          if (code < 0x80) {
            out += static_cast<char>(code);
          } else if (code < 0x800) {
            out += static_cast<char>(0xC0 | (code >> 6));
            out += static_cast<char>(0x80 | (code & 0x3F));
          } else {
            out += static_cast<char>(0xE0 | (code >> 12));
            out += static_cast<char>(0x80 | ((code >> 6) & 0x3F));
            out += static_cast<char>(0x80 | (code & 0x3F));
          }
          break;
        }
        default: return fail("unknown escape");
      }
    }
    return fail("unterminated string");
  }

  bool parse_value(Value& v, int depth) {
    if (depth > kMaxDepth) return fail("nesting too deep");
    skip_ws();
    if (i >= s.size()) return fail("unexpected end of input");
    const char c = s[i];
    if (c == '{') return parse_object(v, depth);
    if (c == '[') return parse_array(v, depth);
    if (c == '"') {
      v.type = Value::Type::String;
      return parse_string(v.str);
    }
    if (c == 't') {
      if (!literal("true")) return false;
      v.type = Value::Type::Bool;
      v.boolean = true;
      return true;
    }
    if (c == 'f') {
      if (!literal("false")) return false;
      v.type = Value::Type::Bool;
      v.boolean = false;
      return true;
    }
    if (c == 'n') {
      if (!literal("null")) return false;
      v.type = Value::Type::Null;
      return true;
    }
    return parse_number(v);
  }

  bool parse_number(Value& v) {
    const char* begin = s.c_str() + i;
    char* end = nullptr;
    const double d = std::strtod(begin, &end);
    if (end == begin) return fail("expected value");
    i += static_cast<std::size_t>(end - begin);
    v.type = Value::Type::Number;
    v.number = d;
    return true;
  }

  bool parse_array(Value& v, int depth) {
    ++i;  // '['
    v.type = Value::Type::Array;
    skip_ws();
    if (i < s.size() && s[i] == ']') {
      ++i;
      return true;
    }
    for (;;) {
      Value item;
      if (!parse_value(item, depth + 1)) return false;
      v.arr.push_back(std::move(item));
      skip_ws();
      if (i >= s.size()) return fail("unterminated array");
      if (s[i] == ',') {
        ++i;
        continue;
      }
      if (s[i] == ']') {
        ++i;
        return true;
      }
      return fail("expected ',' or ']'");
    }
  }

  bool parse_object(Value& v, int depth) {
    ++i;  // '{'
    v.type = Value::Type::Object;
    skip_ws();
    if (i < s.size() && s[i] == '}') {
      ++i;
      return true;
    }
    for (;;) {
      skip_ws();
      std::string key;
      if (!parse_string(key)) return false;
      skip_ws();
      if (i >= s.size() || s[i] != ':') return fail("expected ':'");
      ++i;
      Value item;
      if (!parse_value(item, depth + 1)) return false;
      v.obj.emplace_back(std::move(key), std::move(item));
      skip_ws();
      if (i >= s.size()) return fail("unterminated object");
      if (s[i] == ',') {
        ++i;
        continue;
      }
      if (s[i] == '}') {
        ++i;
        return true;
      }
      return fail("expected ',' or '}'");
    }
  }
};

}  // namespace detail

// text 全体を 1 個の JSON 値として読む(末尾のゴミも拒否)。
inline bool parse(const std::string& text, Value& out, std::string& err) {
  detail::Parser p(text);
  Value v;
  if (!p.parse_value(v, 0)) {
    err = p.err;
    return false;
  }
  p.skip_ws();
  if (p.i != text.size()) {
    err = "parse error at offset " + std::to_string(p.i) + ": trailing characters";
    return false;
  }
  out = std::move(v);
  err.clear();
  return true;
}

}  // namespace jsonmin

#endif  // TPCDAQ_ECC_BRIDGE_JSON_MIN_HPP
