// env_json.ino — Arduino MKR 系 + MKR ENV Shield(ASX00011)
//
// TODO/079 §2.3 / §2.7: 温度・湿度・気圧・照度・UV を 1 秒毎に JSON 1 行で USB シリアルへ
// 流すだけの自前スケッチ(WiFi は使わない —— 裁定 2026-09-15、KISS)。
//
// slowctl 側(src/slowctl/arduino.rs)がこの JSON をそのまま読む。フィールド名・単位・
// 出力周期を変える場合は両方を揃えて直すこと。
//
// 依存ライブラリ: Arduino_MKRENV(Arduino IDE の Library Manager、または
// `arduino-cli lib install "Arduino_MKRENV"` でインストール。tools/arduino_env/README.md 参照)。

#include <Arduino_MKRENV.h>

// slowctl 側の EXPECTED_DEV("mkr_env", src/slowctl/arduino.rs)と一致させること。
const char *DEV_NAME = "mkr_env";

unsigned long seq = 0;

void setup() {
  Serial.begin(115200);
  while (!Serial) {
    ; // USB CDC が確立するまで待つ(MKR1000/MKR1010 系の既知の作法)
  }

  if (!ENV.begin()) {
    // シールドが見つからない場合は 1 秒毎にエラー行を出し続ける
    // (slowctl::arduino::Driver::ident / poll は両方ともこれを Protocol エラーとして扱う)。
    // 復旧はシールドの物理的な再装着 + 再起動のみ(このスケッチは自己修復しない)。
    while (true) {
      Serial.print("{\"dev\":\"");
      Serial.print(DEV_NAME);
      Serial.println("\",\"error\":\"ENV shield not found\"}");
      delay(1000);
    }
  }
}

void loop() {
  float t_c = ENV.readTemperature();
  float rh = ENV.readHumidity();
  // MILLIBAR = hPa と同じ桁(1 mbar = 1 hPa)。既定の readPressure() は kPa なので明示する。
  float p_hpa = ENV.readPressure(MILLIBAR);
  float lux = ENV.readIlluminance();
  float uva = ENV.readUVA();
  float uvb = ENV.readUVB();

  Serial.print("{\"dev\":\"");
  Serial.print(DEV_NAME);
  Serial.print("\",\"seq\":");
  Serial.print(seq);
  Serial.print(",\"t_c\":");
  Serial.print(t_c, 2);
  Serial.print(",\"rh\":");
  Serial.print(rh, 2);
  Serial.print(",\"p_hpa\":");
  Serial.print(p_hpa, 2);
  Serial.print(",\"lux\":");
  Serial.print(lux, 2);
  Serial.print(",\"uva\":");
  Serial.print(uva, 2);
  Serial.print(",\"uvb\":");
  Serial.print(uvb, 2);
  Serial.println("}");

  seq++;
  delay(1000);
}
