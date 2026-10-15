# 145 — CAEN GECO2020(snap)を ELITPC で DT1415ET に繋ぐ(HV の設定はこれで、DAQ は落として)

**Status: COMPLETED**(2026-10-15、ユーザー(インストール・操作)+ Fable(切り分け))

## 事実
- `caengeco2020` 1.16.1 を snap **devmode** で導入(`install.sh` が `snap install --devmode`)。プラグ: home / network / opengl / unity7 / x11(`personal-files` の config-geco は未接続)。devmode なので `/dev/ttyACM0`(dialout、daq は所属)も `/var/lock` も snap 内から使える(確認済み)。
- DT1415ET は USB CDC = `/dev/ttyACM0`(`/dev/serial/by-id/usb-CAEN_SPA_DT1415_HV_Power_Supply-if00`)。slowctl も同じポートを 1 s ごとに開閉している。

## はまった点と解
1. **「Authorization required, but no authorization protocol specified」で窓が開かない** — GNOME が Wayland セッションで、snap の x11 が XWayland のクッキー(`/run/user/1000/.mutter-Xwaylandauth.*`)を使えない。解: `xhost +SI:localuser:daq`(同じマシンの daq にだけ許可 = 実質緩めない)。恒久化: `~/.config/autostart/xhost-local.desktop`(`Exec=xhost +SI:localuser:daq`、desktop-file-validate OK、次回ログインから)。sudo は無関係(アプリは daq のセッションで動いていた)。
2. **「Unable to connect / Some errors occurred」** — 2 つ重なっていた:
   - slowctl が動いたまま(= DAQ を落とさずに GECO を起動)で `/dev/ttyACM0` を取り合っていた。GECO を閉じた後も `/dev/shm/CAENV_LCK_0_0`(CAEN のリンク 0 ロック)が残る。
   - **Port 欄に `/dev/ttyACM0` と書くと失敗する**: libcaenhvwrapper は `/dev/%s` でパスを組むので `/dev//dev/ttyACM0` になる。**`ttyACM0` と書く**。これが本命(GECO がポートに触りもしない症状 = lsof に出ない・ロックも出ない)。
   - 種別は **SMART HV**(DT1415ET の識別名、マニュアル p.16)、接続 **VCP USB**、9600 / 8 / 1 / none、**LBus Address 0**。DT55xx 系の種別(保存キー `DTHV_`)を選ぶと生 USB を探しに行って失敗する。本体は REMOTE。
3. 切り分けの道具: `lsof /dev/ttyACM0`(誰が握っているか)、読み出しだけのプローブ(`$CMD:MON,CH:8,PAR:VMON` → `#CMD:OK,VAL:…`、/tmp/hvprobe_ro.py)、`~/snap/caengeco2020/x1/.config/CAEN/App.conf`(GECO が保存した接続設定)、`strings libcaenhvwrapper.so`。snap の中のプロセスに strace は付かない(ptrace_scope 1、snap-confine は setuid)。

## 運用(ユーザー決定)
HV を設定するときは **`daq_down.sh` → GECO で設定 → GECO を閉じる → `daq_up.sh`**。次の run の run_start.slow に新しい値が残る。GECO を開いたまま DAQ を上げない。

## 実行できなかった操作
sudo(snap install、apt)と GECO の操作はユーザー。strace によるトレースは不可(上記)。
