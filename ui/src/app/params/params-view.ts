/**
 * Parameters ビュー(SPEC v1.33 §3.3 / §8.1、TODO/078 §11 レーン C)。
 *
 * JSON が正、xcfg は生成物(078 §0)。ここは「一様値 + 例外リスト」を編集して
 * `PUT /api/params` に送るだけ —— CompoundConfig の継承解決や xcfg の生成規則は
 * サーバ(`src/params/render.rs`)の仕事で、ここには持ち込まない。
 *
 * # 責務の分担
 *
 * - 読み込みは `ParamsApiService`(閲覧系、認証なし)。`GET /api/params` が 404 なら
 *   まだ何も保存されていない ——「テンプレート名を入れて既定値で作る」導線を出す
 *   (`renderParams()` は current.json が要るので使えない。既定値は
 *   `tests/fixtures/params/params_roundtrip.json` の値そのもの)。
 * - 保存は `RunControlApiService` の token(操作権 1 名モデル)。**失敗はサーバの文言を
 *   そのまま**出す(041 D-1 / silent 失敗を作らない)。
 * - Running 中の判定は **monitor WS の `status.state`**(078 §11 の指定どおり。controller の
 *   `phase` ではない —— root-sink が実際にイベントを積んでいるかどうかを見る)。
 * - スコープ(CoBo/AsAd/AGET/channel)は画面に出さない(078 §0)。`knob-spec.ts` の
 *   `scopeDepth` は Preview のハイライトにだけ使う。
 */
import { Component, computed, inject, signal } from '@angular/core';

import {
  type ParamsInfo,
  type ParamsJson,
  ParamsApiError,
  ParamsApiService,
} from '../api/params-api';
import { RunControlApiService } from '../api/run-control-api';
import { describeError } from '../logbook/logbook-feed';
import { WsClientService } from '../ws/ws-client';
import {
  KNOB_FIELDS,
  KNOB_GROUPS,
  MANAGED_XCFG_ELEMENTS,
  type KnobField,
  flattenKnobs,
  knobField,
  nestKnobs,
} from './knob-spec';

/** 例外テーブルの 1 行(画面編集用。値はすべて文字列 —— 送信時に knob の型へ変換する)。 */
export interface ExceptionDraftRow {
  readonly asad: string;
  readonly aget: string;
  readonly ch: string;
  readonly knob: string;
  readonly value: string;
  readonly note: string;
}

/**
 * テンプレート未設定時の既定値(= `tests/fixtures/params/params_roundtrip.json` の `knobs`、
 * 078 §11「knobs は表の既定値(configure-physics 25 MHz の現行値)で空フォームを作る」)。
 * `knob-spec.spec.ts` 経由ではなくここで直接固定しているので、フィクスチャが動いたら
 * `params-view.spec.ts` がずれを検知する。
 */
export const DEFAULT_KNOBS: Record<string, unknown> = {
  triggerDelay: 1748,
  writingClockFrequency_MHz: 25.0,
  triggerMode: 'external',
  globalThreshold: 1,
  lsbThreshold: 15,
  gain: '120fC',
  triggerInhibition: 'none',
  peakingTime_ns: 232,
  readout: 'full',
  zeroSuppression: { enabled: false, threshold: 600 },
  pulser: { enabled: false, voltage_mV: -1000.0, period_ms: 1000 },
};

function defaultValueText(field: KnobField): string {
  if (field.kind === 'bool') return 'false';
  if (field.choices && field.choices.length > 0) return String(field.choices[0]);
  return '0';
}

function emptyExceptionRow(): ExceptionDraftRow {
  const first = KNOB_FIELDS[0];
  return { asad: '', aget: '', ch: '', knob: first.key, value: defaultValueText(first), note: '' };
}

@Component({
  selector: 'app-params-view',
  templateUrl: './params-view.html',
  styleUrl: './params-view.scss',
})
export class ParamsView {
  private readonly api = inject(ParamsApiService);
  private readonly control = inject(RunControlApiService);
  private readonly ws = inject(WsClientService);

  readonly groups = KNOB_GROUPS;
  readonly fields = KNOB_FIELDS;

  readonly loading = signal(true);
  readonly loadError = signal<string | null>(null);
  /** 直近にサーバから読めた(または保存できた)params。`null` = まだ何も無い(404)。 */
  readonly info = signal<ParamsInfo | null>(null);
  /** フォームを描画してよいか(読み込み済み、または「既定値で作る」を押した)。 */
  readonly draftReady = signal(false);

  /** 「No parameters yet」のテンプレート名入力。 */
  readonly newTemplateInput = signal('');

  /** 編集中の下書き。 */
  readonly templateName = signal('');
  readonly knobValues = signal<Map<string, unknown>>(new Map());
  readonly exceptionsDraft = signal<ExceptionDraftRow[]>([]);
  readonly noteText = signal('');

  readonly hasToken = computed(() => this.control.token() !== null);

  /** SPEC §10.3 の monitor WS status(078 §11: controller の phase ではなくこちらを見る)。 */
  readonly runningNow = computed(() => this.ws.status()?.state === 'running');

  readonly saveBusy = signal(false);
  readonly saveError = signal<string | null>(null);
  readonly saveResult = signal<ParamsInfo | null>(null);

  readonly previewBusy = signal(false);
  readonly previewError = signal<string | null>(null);
  readonly previewText = signal<string | null>(null);
  readonly previewLines = computed(() => (this.previewText() ?? '').split('\n'));

  /** Save が押せない理由(押せるときは `null`)。理由は必ず画面に出す(silent 禁止)。 */
  readonly saveDisabledReason = computed<string | null>(() => {
    if (!this.draftReady()) return 'Load parameters first';
    if (this.templateName().trim() === '') return 'Template name is required';
    if (!this.hasToken()) return 'Acquire control in Run control first';
    if (this.runningNow()) return 'A run is active — cannot change parameters while Running';
    if (this.saveBusy()) return 'Saving…';
    return null;
  });
  readonly saveDisabled = computed(() => this.saveDisabledReason() !== null);

  /** Preview は current.json が無いと render できない(dry run はサーバ保存分を読む)。 */
  readonly previewDisabledReason = computed<string | null>(() => {
    if (this.info() === null) return 'Save once first (Preview renders the saved current.json)';
    if (this.previewBusy()) return 'Rendering…';
    return null;
  });
  readonly previewDisabled = computed(() => this.previewDisabledReason() !== null);

  constructor() {
    void this.load();
  }

  private async load(): Promise<void> {
    this.loading.set(true);
    this.loadError.set(null);
    try {
      const info = await this.api.getParams();
      this.info.set(info);
      if (info !== null) {
        this.loadDraftFrom(info.params);
        this.draftReady.set(true);
      }
    } catch (error) {
      this.loadError.set(error instanceof ParamsApiError ? error.message : describeError(error));
    } finally {
      this.loading.set(false);
    }
  }

  private loadDraftFrom(params: ParamsJson): void {
    this.templateName.set(params.template);
    this.knobValues.set(flattenKnobs(params.knobs));
    this.exceptionsDraft.set(
      params.exceptions.map((exception) => ({
        asad: exception.at.asad?.toString() ?? '',
        aget: exception.at.aget?.toString() ?? '',
        ch: exception.at.ch?.toString() ?? '',
        knob: exception.knob,
        value: String(exception.value),
        note: exception.note ?? '',
      })),
    );
    this.noteText.set(params.note ?? '');
  }

  setNewTemplateInput(event: Event): void {
    this.newTemplateInput.set((event.target as HTMLInputElement).value);
  }

  /** 「Load defaults from template」— knobs は既定値(§0 のとおり、テンプレートは読まない)。 */
  loadDefaults(): void {
    const name = this.newTemplateInput().trim();
    if (name === '') return;
    this.templateName.set(name);
    this.knobValues.set(flattenKnobs(DEFAULT_KNOBS));
    this.exceptionsDraft.set([]);
    this.noteText.set('');
    this.saveResult.set(null);
    this.draftReady.set(true);
  }

  setTemplateName(event: Event): void {
    this.templateName.set((event.target as HTMLInputElement).value);
  }

  setNote(event: Event): void {
    this.noteText.set((event.target as HTMLInputElement).value);
  }

  displayValue(key: string): string {
    const value = this.knobValues().get(key);
    return value === undefined || value === null ? '' : String(value);
  }

  isChecked(key: string): boolean {
    return this.knobValues().get(key) === true;
  }

  /**
   * `<select [value]>` は子 `<option>` がまだ DOM に無い間に設定されると効かない
   * (ブラウザの仕様 — Angular の `@for` で生成した option は親の value 束縛より後に
   * 挿入される)。なので `<option [selected]>` の側で選択状態を作る。
   */
  isEnumSelected(field: KnobField, choice: string | number): boolean {
    return String(choice) === this.displayValue(field.key);
  }

  private setKnob(key: string, value: unknown): void {
    this.knobValues.update((map) => {
      const next = new Map(map);
      next.set(key, value);
      return next;
    });
  }

  private coerceScalar(field: KnobField, raw: string): unknown {
    if (field.kind === 'int') return Number.parseInt(raw, 10);
    if (field.kind === 'real') return Number(raw);
    if (field.kind === 'enum' && field.choices && typeof field.choices[0] === 'number') {
      return Number(raw);
    }
    return raw;
  }

  onBoolChange(field: KnobField, event: Event): void {
    this.setKnob(field.key, (event.target as HTMLInputElement).checked);
  }

  onScalarChange(field: KnobField, event: Event): void {
    const raw = (event.target as HTMLInputElement | HTMLSelectElement).value;
    this.setKnob(field.key, this.coerceScalar(field, raw));
  }

  exceptionField(row: ExceptionDraftRow): KnobField | undefined {
    return knobField(row.knob);
  }

  isExceptionKnobSelected(row: ExceptionDraftRow, key: string): boolean {
    return row.knob === key;
  }

  isExceptionValueSelected(row: ExceptionDraftRow, choice: string | number): boolean {
    return String(choice) === row.value;
  }

  private updateExceptionRow(index: number, patch: Partial<ExceptionDraftRow>): void {
    this.exceptionsDraft.update((rows) =>
      rows.map((row, i) => (i === index ? { ...row, ...patch } : row)),
    );
  }

  addException(): void {
    this.exceptionsDraft.update((rows) => [...rows, emptyExceptionRow()]);
  }

  removeException(index: number): void {
    this.exceptionsDraft.update((rows) => rows.filter((_, i) => i !== index));
  }

  /** 「Reset to uniform」— 例外を全部消す(一様値だけに戻す)。 */
  resetToUniform(): void {
    this.exceptionsDraft.set([]);
  }

  updateAt(index: number, key: 'asad' | 'aget' | 'ch', event: Event): void {
    const raw = (event.target as HTMLInputElement).value;
    this.updateExceptionRow(index, { [key]: raw } as Partial<ExceptionDraftRow>);
  }

  updateExceptionValue(index: number, event: Event): void {
    const raw = (event.target as HTMLInputElement | HTMLSelectElement).value;
    this.updateExceptionRow(index, { value: raw });
  }

  updateExceptionNote(index: number, event: Event): void {
    const raw = (event.target as HTMLInputElement).value;
    this.updateExceptionRow(index, { note: raw });
  }

  onExceptionKnobChange(index: number, event: Event): void {
    const key = (event.target as HTMLSelectElement).value;
    const field = knobField(key);
    this.updateExceptionRow(index, { knob: key, value: field ? defaultValueText(field) : '' });
  }

  private parseExceptionValue(row: ExceptionDraftRow): unknown {
    const field = knobField(row.knob);
    if (field === undefined) return row.value;
    if (field.kind === 'bool') return row.value === 'true';
    if (field.kind === 'int') return Number.parseInt(row.value, 10);
    if (field.kind === 'real') return Number(row.value);
    if (field.kind === 'enum' && field.choices && typeof field.choices[0] === 'number') {
      return Number(row.value);
    }
    return row.value;
  }

  private buildAt(row: ExceptionDraftRow): { asad?: number; aget?: number; ch?: number } {
    const at: { asad?: number; aget?: number; ch?: number } = {};
    if (row.asad.trim() !== '') at.asad = Number.parseInt(row.asad, 10);
    if (row.aget.trim() !== '') at.aget = Number.parseInt(row.aget, 10);
    if (row.ch.trim() !== '') at.ch = Number.parseInt(row.ch, 10);
    return at;
  }

  private buildParamsJson(): ParamsJson {
    const knobs = nestKnobs(this.knobValues());
    const exceptions = this.exceptionsDraft().map((row) => {
      const note = row.note.trim();
      return {
        at: this.buildAt(row),
        knob: row.knob,
        value: this.parseExceptionValue(row),
        ...(note === '' ? {} : { note }),
      };
    });
    const note = this.noteText().trim();
    return {
      schema: 'tpcdaq-params/1',
      template: this.templateName().trim(),
      knobs,
      exceptions,
      ...(note === '' ? {} : { note }),
    };
  }

  async save(): Promise<void> {
    if (this.saveDisabled()) return;
    const token = this.control.token();
    if (token === null) return; // saveDisabledReason が防いでいるはずだが、型のために残す。
    this.saveBusy.set(true);
    this.saveError.set(null);
    try {
      const result = await this.api.putParams(token, this.buildParamsJson());
      this.info.set(result);
      this.saveResult.set(result);
      this.loadDraftFrom(result.params);
      // 保存内容が変わったので古い preview を残さない(黙って古い xcfg を見せない)。
      this.previewText.set(null);
      this.previewError.set(null);
    } catch (error) {
      this.saveError.set(error instanceof ParamsApiError ? error.message : describeError(error));
    } finally {
      this.saveBusy.set(false);
    }
  }

  async preview(): Promise<void> {
    if (this.previewDisabled()) return;
    this.previewBusy.set(true);
    this.previewError.set(null);
    try {
      const result = await this.api.renderParams();
      this.previewText.set(result.configure_xcfg);
    } catch (error) {
      this.previewError.set(error instanceof ParamsApiError ? error.message : describeError(error));
      this.previewText.set(null);
    } finally {
      this.previewBusy.set(false);
    }
  }

  /** 管理ノブの xcfg 要素名を含む行だけハイライトする(diff の代替、078 §11 の逸脱)。 */
  isManagedLine(line: string): boolean {
    return MANAGED_XCFG_ELEMENTS.some((name) => line.includes(name));
  }
}
