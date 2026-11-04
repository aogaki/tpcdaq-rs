# 196 — v_drift(z の目盛り)の絶対較正: 同じ条件の α 線源 run

**Status: WAITING(チャンバーを開ける日に、ユーザーの run)**(2026-11-02 起票 Opus、Fable 裁定 = 192「Fable 所見」§3)

## 動機
run 209 の条件(CO2 198 mbar、ドリフト 1,600 V / 20 cm、E/p = 0.40)は v_drift の表(E/p 0.54–1.05)の外挿。147 で v ∝ E の相対の線形性(± 6 %)は実測済みなので、外挿の誤差は ≲ 10 % と見る(選別・表示・当てはめの前提にはしない)。ただし **E_n と開き角を数字で出す前に絶対値が要る**。順方向の当てはめ(197)は v_drift を決められない(066 の厳密縮退: 直線は strip × cell でも直線)。陰極端(z-edge)の方法は、窓(512 cell × 0.234 mm ≈ 120 mm)< ドリフト長 200 mm で陰極が窓に入らないので使えない。

## run の手順(ユーザー)
- ~~条件を run 209 と同じに: … peaking 502 ns …(params `elinp__818acd22`)~~ **条件 = 今後の物理 run と同じに(2026-11-04 ユーザー「今後は 232 ns で行こうと思う」)**: CO2 198 mbar、ドリフト 1,600 V、GEM 260 V、12.5 MHz、**peaking 232 ns**、120 fC、delay 2500、トリガ = N1068GE の OR(params **`elinp__8efb10d4`**)。v_drift は peaking に依らないので 502 ns の 209 にも使える(z の目盛りは電子のドリフトで決まる)。解析の analyzeUVW は `stripQMin = 1100`(207)。HV ON から 15 分待ってから開始。
- α 線源(²²⁸Th — stoppedAlphas.C の既知の線)をケージの中の端に置き、**1 時間**。
- 湿度の件があるので、中性子の run の最中には開けない。**チャンバーを開ける機会に**。

## 解析(Opus、run の後)
- analyzeUVW → makeTracks → stoppedAlphas.C に **mmPerCell の走査**を足す: 止まった α の 3D 飛程 = √(Δxy² + (Δcell × mmPerCell)²) から ASTAR でエネルギー → 既知の線に合う mmPerCell を、**dip 角の 3 ビンでエネルギーが一致する(± 3 %)値**として決める(dip が 0 の飛跡は mmPerCell に依らず、急な飛跡ほど効く)。
- 受け入れ: mmPerCell とその誤差、analyzeUVW の外挿値(0.2338 mm / cell)との比。

## 次
較正値が出たら、173 の運動学(E_n、開き角)と 197–198 の当てはめの z に入れる。
