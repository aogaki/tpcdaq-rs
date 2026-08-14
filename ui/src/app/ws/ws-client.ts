/**
 * monitor の WS(SPEC §10)に繋ぐクライアント。Angular signals で最新値を公開する。
 *
 * 責務はこれだけ:
 * - 接続 / 自動再接続(指数バックオフ + ジッタ)/ 手動再接続
 * - `subscribe` の送信(既定は waveforms OFF。028 の波形ビューが表示中だけ ON にする)
 * - 最新の `meta` / `status` / `run` / `Uvw` / `Histo*` を保持
 * - 受けたもの・落としたものを**全部数える**(silent 禁止)
 *
 * 集計や描画はしない(それは 028/029 の仕事)。
 */
import { Injectable, computed, signal } from '@angular/core';

import {
  DEFAULT_STREAMS,
  type MetaMessage,
  type RunMessage,
  type StatusMessage,
  type StreamSet,
  encodeSubscribe,
  parseJsonMessage,
} from './json';
import { STATUS_STALE_MS, type LinkState, reconnectDelayMs, statusHealth } from './state';
import {
  type Histo1dMessage,
  type Histo2dMessage,
  type UvwMessage,
  type WaveformsMessage,
  decodeBinary,
} from './wire';

/** 使う分だけの `WebSocket`(テストから fake を差し込めるように狭く取る)。 */
export interface WsSocket {
  binaryType: string;
  send(data: string): void;
  close(): void;
  onopen: (() => void) | null;
  onmessage: ((event: { data: unknown }) => void) | null;
  onclose: (() => void) | null;
  onerror: (() => void) | null;
}

export type SocketFactory = (url: string) => WsSocket;

export type HistoMessage = Histo1dMessage | Histo2dMessage;

/** 受けたもの・落としたものの内訳(status バーに出す)。 */
export interface WsCounters {
  readonly meta: number;
  readonly status: number;
  readonly run: number;
  readonly uvw: number;
  readonly waveforms: number;
  readonly histo1d: number;
  readonly histo2d: number;
  /** バイナリのデコード失敗(magic / version / 長さ)。 */
  readonly decodeErrors: number;
  /** JSON のパース失敗(壊れた JSON / 型違い)。 */
  readonly jsonErrors: number;
  /** 知らない `type` の JSON(前方互換で無視した数)。 */
  readonly unknownJsonTypes: number;
  /** 既知メッセージ内の知らないキーの数(前方互換で無視した数)。 */
  readonly unknownJsonKeys: number;
  /** 自動再接続した回数。 */
  readonly reconnects: number;
}

const ZERO_COUNTERS: WsCounters = {
  meta: 0,
  status: 0,
  run: 0,
  uvw: 0,
  waveforms: 0,
  histo1d: 0,
  histo2d: 0,
  decodeErrors: 0,
  jsonErrors: 0,
  unknownJsonTypes: 0,
  unknownJsonKeys: 0,
  reconnects: 0,
};

/** `health` を進めるための時計の刻み(staleness の分解能)。 */
const CLOCK_TICK_MS = 500;

@Injectable({ providedIn: 'root' })
export class WsClientService {
  /** テスト・特殊環境用の差し替え口(既定は本物の `WebSocket`)。 */
  socketFactory: SocketFactory = (url) => new WebSocket(url) as unknown as WsSocket;
  /** 時計(テストから固定できるように)。 */
  now: () => number = () => Date.now();
  /** バックオフのジッタ用乱数。 */
  random: () => number = Math.random;

  readonly url = signal<string>('');
  readonly link = signal<LinkState>('disconnected');

  readonly meta = signal<MetaMessage | null>(null);
  readonly status = signal<StatusMessage | null>(null);
  readonly run = signal<RunMessage | null>(null);

  /** 面(0=U,1=V,2=W)毎の最新 Uvw。028 の モニタビューが読む。 */
  readonly uvwByPlane = signal<readonly (UvwMessage | null)[]>([null, null, null]);
  /** 最新の波形(購読 ON のときだけ届く)。 */
  readonly waveforms = signal<WaveformsMessage | null>(null);
  /** SPEC §5.2 の id をキーにした最新ヒスト。 */
  readonly histos = signal<ReadonlyMap<number, HistoMessage>>(new Map());

  readonly streams = signal<StreamSet>(DEFAULT_STREAMS);
  readonly counters = signal<WsCounters>(ZERO_COUNTERS);
  /** 直近に落としたものの一言説明(status バーの tooltip 用)。 */
  readonly lastIssue = signal<string | null>(null);

  private readonly connectedAtMs = signal<number | null>(null);
  private readonly lastStatusAtMs = signal<number | null>(null);
  /** `health` を時間で再評価させるための刻み。 */
  private readonly clock = signal<number>(0);

  /** SPEC §10.3 の status が届いているか(未接続と区別する)。 */
  readonly health = computed(() =>
    statusHealth({
      link: this.link(),
      connectedAtMs: this.connectedAtMs(),
      lastStatusAtMs: this.lastStatusAtMs(),
      nowMs: this.clock(),
      staleAfterMs: STATUS_STALE_MS,
    }),
  );

  private socket: WsSocket | null = null;
  private attempt = 0;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private clockTimer: ReturnType<typeof setInterval> | null = null;
  /** 手動 `disconnect()` 後は再接続しない。 */
  private wanted = false;

  /** 接続を開始する(以後、切れたら自動で繋ぎ直す)。 */
  connect(url: string): void {
    this.url.set(url);
    this.wanted = true;
    this.attempt = 0;
    this.startClock();
    this.open();
  }

  /** 手動で今すぐ繋ぎ直す。 */
  reconnectNow(): void {
    if (!this.wanted) this.wanted = true;
    this.clearReconnectTimer();
    this.attempt = 0;
    this.startClock();
    this.open();
  }

  /** 手動で止める(自動再接続もしない)。 */
  disconnect(): void {
    this.wanted = false;
    this.clearReconnectTimer();
    this.stopClock();
    this.closeSocket();
    this.link.set('disconnected');
    this.connectedAtMs.set(null);
  }

  /**
   * 波形ストリームの ON/OFF(028 の波形ビューが表示中だけ ON にする)。
   * 値が変わったときだけ `subscribe` を送り直す。
   */
  setWaveforms(on: boolean): void {
    const current = this.streams();
    if (current.waveforms === on) return;
    this.streams.set({ ...current, waveforms: on });
    if (this.link() === 'connected') this.sendSubscribe();
  }

  // -------------------------------------------------------------------
  // 内部
  // -------------------------------------------------------------------

  private open(): void {
    this.closeSocket();
    this.link.set('connecting');
    const socket = this.socketFactory(this.url());
    socket.binaryType = 'arraybuffer';
    socket.onopen = () => this.onOpen();
    socket.onmessage = (event) => this.onMessage(event.data);
    socket.onclose = () => this.onClose();
    socket.onerror = () => this.note('WebSocket error');
    this.socket = socket;
  }

  private onOpen(): void {
    this.attempt = 0;
    this.connectedAtMs.set(this.now());
    this.lastStatusAtMs.set(null);
    this.link.set('connected');
    this.tickClock();
    this.sendSubscribe();
  }

  private onClose(): void {
    this.socket = null;
    this.link.set('disconnected');
    this.connectedAtMs.set(null);
    this.tickClock();
    if (!this.wanted) return;

    this.attempt += 1;
    const delay = reconnectDelayMs(this.attempt, this.random);
    this.clearReconnectTimer();
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.bump({ reconnects: 1 });
      this.open();
    }, delay);
  }

  private sendSubscribe(): void {
    const socket = this.socket;
    if (!socket) return;
    try {
      socket.send(encodeSubscribe(this.streams()));
    } catch (error) {
      this.note(`cannot send subscribe: ${String(error)}`);
    }
  }

  private onMessage(data: unknown): void {
    if (typeof data === 'string') {
      this.onText(data);
      return;
    }
    if (data instanceof ArrayBuffer) {
      this.onBinary(data);
      return;
    }
    // binaryType = 'arraybuffer' にしているので本来ここには来ない。
    this.bump({ decodeErrors: 1 });
    this.note(`unexpected WS frame type: ${typeof data}`);
  }

  private onBinary(buffer: ArrayBuffer): void {
    const message = decodeBinary(buffer);
    switch (message.kind) {
      case 'uvw': {
        const planes = [...this.uvwByPlane()];
        if (message.plane < planes.length) planes[message.plane] = message;
        this.uvwByPlane.set(planes);
        this.bump({ uvw: 1 });
        return;
      }
      case 'waveforms':
        this.waveforms.set(message);
        this.bump({ waveforms: 1 });
        return;
      case 'histo1d':
      case 'histo2d': {
        const histos = new Map(this.histos());
        histos.set(message.id, message);
        this.histos.set(histos);
        this.bump(message.kind === 'histo1d' ? { histo1d: 1 } : { histo2d: 1 });
        return;
      }
      case 'error':
        this.bump({ decodeErrors: 1 });
        this.note(`binary ${message.reason}: ${message.detail}`);
        return;
    }
  }

  private onText(text: string): void {
    const parsed = parseJsonMessage(text);
    switch (parsed.kind) {
      case 'meta':
        this.meta.set(parsed.meta);
        this.bump({ meta: 1, unknownJsonKeys: parsed.unknownKeys.length });
        this.noteUnknownKeys('meta', parsed.unknownKeys);
        return;
      case 'status':
        this.status.set(parsed.status);
        this.lastStatusAtMs.set(this.now());
        this.tickClock();
        this.bump({ status: 1, unknownJsonKeys: parsed.unknownKeys.length });
        this.noteUnknownKeys('status', parsed.unknownKeys);
        return;
      case 'run':
        this.run.set(parsed.run);
        this.bump({ run: 1, unknownJsonKeys: parsed.unknownKeys.length });
        this.noteUnknownKeys('run', parsed.unknownKeys);
        return;
      case 'unknown-type':
        this.bump({ unknownJsonTypes: 1 });
        this.note(`unknown JSON type: ${parsed.type}`);
        return;
      case 'error':
        this.bump({ jsonErrors: 1 });
        this.note(`JSON ${parsed.reason}: ${parsed.detail}`);
        return;
    }
  }

  private noteUnknownKeys(where: string, keys: readonly string[]): void {
    if (keys.length === 0) return;
    this.note(`unknown ${where} keys: ${keys.join(', ')}`);
  }

  private bump(delta: Partial<Record<keyof WsCounters, number>>): void {
    const current = this.counters();
    const next: Record<string, number> = { ...current };
    for (const [key, value] of Object.entries(delta)) {
      if (value) next[key] = (next[key] ?? 0) + value;
    }
    this.counters.set(next as unknown as WsCounters);
  }

  private note(message: string): void {
    this.lastIssue.set(message);
  }

  private startClock(): void {
    this.tickClock();
    if (this.clockTimer !== null) return;
    this.clockTimer = setInterval(() => this.tickClock(), CLOCK_TICK_MS);
  }

  private stopClock(): void {
    if (this.clockTimer === null) return;
    clearInterval(this.clockTimer);
    this.clockTimer = null;
  }

  private tickClock(): void {
    this.clock.set(this.now());
  }

  private clearReconnectTimer(): void {
    if (this.reconnectTimer === null) return;
    clearTimeout(this.reconnectTimer);
    this.reconnectTimer = null;
  }

  private closeSocket(): void {
    const socket = this.socket;
    if (!socket) return;
    // 自分で閉じるときは onclose を鳴らさない(自動再接続の二重起動を防ぐ)。
    socket.onopen = null;
    socket.onmessage = null;
    socket.onclose = null;
    socket.onerror = null;
    this.socket = null;
    try {
      socket.close();
    } catch (error) {
      this.note(`cannot close the socket: ${String(error)}`);
    }
  }
}
