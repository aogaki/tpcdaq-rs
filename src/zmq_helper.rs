//! ZMQ 配線ヘルパ(有限 HWM 方針の単一チョークポイント)。SPEC §1.4。
//!
//! # なぜ有限 HWM か(delila-rs との意図的な差分)
//!
//! delila-rs は全ソケット **HWM=0(無制限バッファ)**で「絶対に落とさない」を実現している。
//! tpcdaq-rs はこれを踏襲**しない**。ELITPC は定常 ~111 MB/s で、無制限バッファは
//! 「ディスクが一瞬でも遅れたらメモリが際限なく膨らみ、最後は OOM でプロセスごと消える」
//! = 最悪の形のデータ喪失になるからである。
//!
//! 代わりに SPEC §1.4 の規約を採る:
//!
//! 1. ロスレスリンクは**有限 HWM(既定 [`DEFAULT_HWM`] = 1000 メッセージ)+ 受け側の有界内部キュー**。
//!    一時的な停滞(ディスクスパイク等)はこのバッファが吸収する。
//! 2. バッファが埋まれば PUSH の send がブロックする = **背圧**。これがロスレスの実装手段。
//! 3. receiver の内部キューまで埋まったら、それはシステム過負荷 = **Error 状態**として
//!    可視化し run を異常停止する。「静かに間引いて run を続ける」ことはしない。
//! 4. モニタ系(PUB/SUB)は間引いてよいが、ドロップは数えて可視化する。
//!
//! # なぜヘルパか
//!
//! 「全ソケットが有限 HWM を通っている」を grep 一発で保証できる場所を 1 つに絞るため。
//! 新しいソケットを足すときにここを通し忘れると、その 1 本だけ libzmq 既定値のまま
//! 走ることになり、方針が静かに破れる。

/// ロスレスリンクの既定 HWM(メッセージ数、SPEC §1.4-2)。
///
/// 目標レートの 2 秒分以上を吸収できることが選定基準。実測に基づく調整は設定で行う。
pub const DEFAULT_HWM: i32 = 1000;

/// 明示した HWM を送受両方向に設定する。
///
/// `hwm <= 0` は無制限(libzmq の HWM=0)になり方針が破れるので拒否する。
/// HWM は **bind / connect の前**に設定しなければ効かない。
pub fn apply_hwm(socket: &zmq::Socket, hwm: i32) -> zmq::Result<()> {
    if hwm <= 0 {
        // SPEC §1.4: 無制限バッファは選ばない。
        return Err(zmq::Error::EINVAL);
    }
    socket.set_sndhwm(hwm)?;
    socket.set_rcvhwm(hwm)
}

/// PUSH(送信側)に既定の有限 HWM を設定する。満杯なら send がブロック = 背圧。
pub fn apply_push_hwm(socket: &zmq::Socket) -> zmq::Result<()> {
    socket.set_sndhwm(DEFAULT_HWM)
}

/// PULL(受信側)に既定の有限 HWM を設定する。
pub fn apply_pull_hwm(socket: &zmq::Socket) -> zmq::Result<()> {
    socket.set_rcvhwm(DEFAULT_HWM)
}

/// PUB(モニタ配信)に既定の有限 HWM を設定する。満杯なら **落とす**(SPEC §1.4-4)。
/// 落とした数は呼び手が数えて可視化すること(silent にしない — CLAUDE.md)。
pub fn apply_pub_hwm(socket: &zmq::Socket) -> zmq::Result<()> {
    socket.set_sndhwm(DEFAULT_HWM)
}

/// SUB(モニタ受信)に既定の有限 HWM を設定する。
/// 取りこぼしは `sequence_number` のギャップとして検出する(SPEC §2.2)。
pub fn apply_sub_hwm(socket: &zmq::Socket) -> zmq::Result<()> {
    socket.set_rcvhwm(DEFAULT_HWM)
}

// ---------------------------------------------------------------------
// テスト(仕様書 — 先に書く)
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn default_hwm_is_finite_and_matches_the_spec() {
        // SPEC §1.4-2: ロスレスリンクは有限 HWM(既定 1000 メッセージ)。
        // delila-rs の HWM=0(無制限)とは意図的に異なる。
        assert_eq!(DEFAULT_HWM, 1000);
        const { assert!(DEFAULT_HWM > 0, "HWM=0 は無制限 = メモリ暴走(SPEC §1.4)") };
    }

    #[test]
    fn each_socket_kind_gets_the_finite_default_on_its_own_direction() {
        let ctx = zmq::Context::new();

        let push = ctx.socket(zmq::PUSH).unwrap();
        apply_push_hwm(&push).unwrap();
        assert_eq!(push.get_sndhwm().unwrap(), DEFAULT_HWM);

        let pull = ctx.socket(zmq::PULL).unwrap();
        apply_pull_hwm(&pull).unwrap();
        assert_eq!(pull.get_rcvhwm().unwrap(), DEFAULT_HWM);

        let publisher = ctx.socket(zmq::PUB).unwrap();
        apply_pub_hwm(&publisher).unwrap();
        assert_eq!(publisher.get_sndhwm().unwrap(), DEFAULT_HWM);

        let subscriber = ctx.socket(zmq::SUB).unwrap();
        apply_sub_hwm(&subscriber).unwrap();
        assert_eq!(subscriber.get_rcvhwm().unwrap(), DEFAULT_HWM);
    }

    #[test]
    fn explicit_hwm_sets_both_directions() {
        let ctx = zmq::Context::new();
        let sock = ctx.socket(zmq::PUSH).unwrap();
        apply_hwm(&sock, 2).unwrap();
        assert_eq!(sock.get_sndhwm().unwrap(), 2);
        assert_eq!(sock.get_rcvhwm().unwrap(), 2);
    }

    /// 無制限(0)を明示要求しても通してしまうと単一チョークポイントの意味がない。
    #[test]
    fn unlimited_hwm_is_rejected() {
        let ctx = zmq::Context::new();
        let sock = ctx.socket(zmq::PUSH).unwrap();
        assert!(apply_hwm(&sock, 0).is_err());
        assert!(apply_hwm(&sock, -1).is_err());
    }
}
