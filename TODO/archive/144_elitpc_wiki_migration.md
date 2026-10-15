# 144 — 旧 TPC 機の PukiWiki(ELI-TPC Wiki)を ELITPC へ移設

**Status: COMPLETED**(2026-10-15 Fable + ユーザー(sudo)。ユーザー「var 以下に wiki か何かのファイルがそのままあります。移すことは現実的ですか?」→ 案 1(新版に載せ替え)を選択)

## 出所
旧 DELILA-Monitor 機(Ubuntu 20.04)のフルバックアップ `~/tmp/TPC_bu.tgz`(Mac、展開済み `~/tmp/`)。wiki = `/var/www/html/tpc` = **PukiWiki 1.5.1**(英語 UI、ページ 49、添付 282 / 1.4 GB、サイト名 ELI-TPC Wiki、トップページ ELI-TPC)。MySQL(199 MB)は DELILA-Monitor 用で無関係。旧機は lib を直接パッチした spam filter + reCAPTCHA 入り(持ち込まず — LAN 内運用)。

## 手順(実施済み)
1. ユーザー: `sudo apt install apache2 php libapache2-mod-php php-mbstring`(PHP 8.5.4 / Apache 2.4.66)。
2. データ rsync(Mac → ELITPC `~/test/wiki_stage/`、LAN + VPN ≈ 11 MB/s、1.55 GB、`wiki attach backup counter diff`)。
3. PukiWiki 1.5.4 を GitHub タグ r1_5_4 から取得(ユーザー許可、429 kB、osdn は名前解決不可)。全 PHP を `php -l`(8.5)で lint: 実害なし(mbstring ポリフィルのみ、未使用)。
4. 設定の移植(`pukiwiki.ini.php` を 1.5.4 の雛形に旧値を写す): `page_title`、`adminpass`(ハッシュそのまま)、`modifier`、`modifierlink`(新ホスト)、`LANG en`、`PKWK_ALLOW_JAVASCRIPT 0`、`defaultpage ELI-TPC`、`nowikiname 1`。スキンは 1.5.4 既定 + 旧ロゴ(`image/bha.jpg`)。
5. ユーザー: `sudo mkdir /srv/wiki && chown daq` → 私が `cp -a site/. /srv/wiki/` → ユーザー: `wiki.conf`(DocumentRoot /srv/wiki、`wiki backup cache counter diff lib` は配信拒否)を有効化、`000-default` を無効化、データ dir を `www-data` に。
6. 確認: `http://192.168.144.220/` 200(タイトル「ELI-TPC - ELI-TPC Wiki」)、ページ一覧、添付(IMG_9523.JPG 1.5 MB、image/jpeg)200、`/wiki/` 403、新規ページの作成 → 表示 → 削除(RecentDeleted に記録)、Apache の error.log に PHP の警告なし。SandBox は旧来 `#freeze`(凍結、管理パスワードで解除)。Mac(VPN)からも 200。

## 運用メモ
- URL: `http://192.168.144.220/`(研究所 LAN / VPN 内のみ、認証は PukiWiki の管理パスワード = 旧機と同じ)。
- 置き場 `/srv/wiki`(コード = daq 所有、データ dir = www-data 所有)。バックアップは `/srv/wiki/{wiki,attach,backup,counter,diff}` を tar するだけ(DB 無し)。
- 旧機の `~/tmp/` 展開物は Mac にあるだけ(リポ外)。`TPC_bu.tgz` の他の中身(DELILA controller の Angular、daqmw、MySQL)は未移設。
- 作業ファイル: ELITPC `~/test/wiki_stage/`(rsync 元・site の複製・`wiki.conf`・`wiki_sudo.txt`)。不要になったら消してよい(1.5 GB)。

## 実行できなかった操作
sudo を要する 4 段はユーザーが実行(apt、mkdir/chown、a2ensite/reload、chown www-data)。
