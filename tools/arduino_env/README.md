# Arduino MKR ENV Shield — 温度・湿度・気圧・照度・UV JSON ロガー

TODO/079 §2.3 / §2.7 の自前スケッチ。ボードは Arduino MKR 系(MKR1000 / MKR1010 等)+
MKR ENV Shield(ASX00011)。WiFi は使わない —— USB シリアルに 1 秒毎に JSON 1 行を
流すだけの最小スケッチ(裁定 2026-09-15、KISS)。

`slowctl`(`src/slowctl/arduino.rs`、TOML `type = "arduino_env"`)がこの出力をそのまま読む。

## 配線

MKR ボードに ENV Shield を差し込むだけ(追加配線なし)。

## 書き込み(`arduino-cli`)

```sh
arduino-cli core update-index
arduino-cli core install arduino:samd
arduino-cli lib install "Arduino_MKRENV"
arduino-cli compile --fqbn arduino:samd:mkr1000 tools/arduino_env/env_json.ino
arduino-cli upload -p /dev/serial/by-id/usb-Arduino... --fqbn arduino:samd:mkr1000 \
    tools/arduino_env/env_json.ino
```

FQBN は実機のボードに合わせる(`arduino-cli board list` で確認できる)。
例: MKR1000 = `arduino:samd:mkr1000`、MKR WiFi 1010 = `arduino:samd:mkrwifi1010`。

Arduino IDE を使う場合は「ボードマネージャ」で Arduino SAMD Boards を、
「ライブラリマネージャ」で `Arduino_MKRENV` をインストールしてから
`env_json.ino` を開いて書き込む。

## `/dev/serial/by-id/` の見つけ方

```sh
ls -l /dev/serial/by-id/
```

TOML の `port` にはこの symlink を書く(USB を挿し直しても `/dev/ttyACM*` の番号が
変わらず壊れない —— TODO/079 §2.1)。個体シリアル番号を持たない安価な変換器では
`/dev/serial/by-path/` を使う(`config::is_stable_serial_port` はどちらも受理する)。

## 出力フォーマット

成功時(1 秒毎、小数 2 桁、`seq` は起動からの通番):

```json
{"dev":"mkr_env","seq":123,"t_c":24.31,"rh":41.2,"p_hpa":1003.7,"lux":120.5,"uva":0.12,"uvb":0.03}
```

`ENV.begin()` に失敗した場合(1 秒毎、シールドを挿し直して再起動するまで続く):

```json
{"dev":"mkr_env","error":"ENV shield not found"}
```

## slowctl 側の読み方(参考、実装は `src/slowctl/arduino.rs`)

- `ident`: 起動直後は出力の途中から読み始めることがあるので、最大 3 行まで読んで
  `"dev":"mkr_env"` を確認する。`error` 行なら識別失敗、`dev` が違えば別機器を読んでいる
  ということ(`/dev/serial/by-id/` を使っていれば起きないはずの誤り)。
- `poll`: バッファに溜まった行を全部読み、**最後の完全な行だけ**を使う(poll 周期が
  1 Hz とずれても古い値に固定されない)。`t_c`/`p_hpa` は単位そのまま(`degC`/`hPa`)、
  `lux` はそのまま、`uva`/`uvb` は較正単位が無いので `raw`。
