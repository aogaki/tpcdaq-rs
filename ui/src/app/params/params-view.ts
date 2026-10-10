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
 *   まだ何も保存されていない ——「テンプレートを選んで読む」導線を出す。ノブの初期値は
 *   **テンプレート xcfg から抽出したもの**(`GET /api/params/defaults`、TODO/111 §5.0)で、
 *   UI に既定値は焼き込まない(mini = 100 MHz / ELITPC = 25 MHz が混ざる事故を防ぐ)。
 * - テンプレートに無い葉(`unset`)・型や範囲に収まらない葉(`unsupported`)は**空欄**にして
 *   理由を出し、埋まるまで Save を無効にする(黙って既定値を入れない)。
 * - 保存は `RunControlApiService` の token(操作権 1 名モデル)。**失敗はサーバの文言を
 *   そのまま**出す(041 D-1 / silent 失敗を作らない)。
 * - Running 中の判定は **monitor WS の `status.state`**(078 §11 の指定どおり。controller の
 *   `phase` ではない —— root-sink が実際にイベントを積んでいるかどうかを見る)。
 * - スコープ(CoBo/AsAd/AGET/channel)は画面に出さない(078 §0)。`knob-spec.ts` の
 *   `scopeDepth` は Preview のハイライトにだけ使う。
 */
import { NgTemplateOutlet } from '@angular/common';
import { Component, computed, inject, signal } from '@angular/core';

import {
  type ExceptionRow,
  type ParamsInfo,
  type ParamsJson,
  type ParamsUnsupported,
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

function defaultValueText(field: KnobField): string {
  if (field.kind === 'bool') return 'false';
  if (field.choices && field.choices.length > 0) return String(field.choices[0]);
  return '0';
}

function emptyExceptionRow(): ExceptionDraftRow {
  const first = KNOB_FIELDS[0];
  return { asad: '', aget: '', ch: '', knob: first.key, value: defaultValueText(first), note: '' };
}

function exceptionRows(exceptions: readonly ExceptionRow[]): ExceptionDraftRow[] {
  return exceptions.map((exception) => ({
    asad: exception.at.asad?.toString() ?? '',
    aget: exception.at.aget?.toString() ?? '',
    ch: exception.at.ch?.toString() ?? '',
    knob: exception.knob,
    value: String(exception.value),
    note: exception.note ?? '',
  }));
}

@Component({
  selector: 'app-params-view',
  imports: [NgTemplateOutlet],
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

  /** 「Load from template」で読むテンプレート名(select / 手入力のどちらでもここに入る)。 */
  readonly newTemplateInput = signal('');
  /** `GET /api/params/templates` の一覧。`null` = 読めていない(理由は `templatesError`)。 */
  readonly templates = signal<string[] | null>(null);
  readonly templatesError = signal<string | null>(null);
  readonly defaultsBusy = signal(false);
  readonly defaultsError = signal<string | null>(null);
  /** 下書きに変更があるまま読み直そうとした —— 確認待ち(捨ててよいか)。 */
  readonly confirmReload = signal(false);

  /** 直近に読んだテンプレートの `unset`(テンプレートに要素が無い葉、`.` 区切り)。 */
  readonly unsetLeaves = signal<readonly string[]>([]);
  /** 直近に読んだテンプレートの `unsupported`(要素はあるが型・範囲外 / 写らない)。 */
  readonly unsupported = signal<readonly ParamsUnsupported[]>([]);
  /** 「下書きに変更があるか」の基準(読み込み / 保存直後のスナップショット)。 */
  private readonly baseline = signal('');

  /** 編集中の下書き。 */
  readonly templateName = signal('');
  readonly knobValues = signal<Map<string, unknown>>(new Map());
  readonly exceptionsDraft = signal<ExceptionDraftRow[]>([]);
  readonly noteText = signal('');

  /** 下書きの全体を比較用の文字列にしたもの(Map は順序に依らないよう key で並べる)。 */
  private readonly snapshot = computed(() =>
    JSON.stringify([
      this.templateName(),
      [...this.knobValues()].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)),
      this.exceptionsDraft(),
      this.noteText(),
    ]),
  );
  readonly draftDirty = computed(() => this.snapshot() !== this.baseline());

  /** 値が入っていない葉(テンプレートに無い / 型外 / 手で空にした)。Save の前に埋める必要がある。 */
  readonly blankKeys = computed(() =>
    KNOB_FIELDS.map((field) => field.key).filter((key) => !this.knobValues().has(key)),
  );

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
    const blank = this.blankKeys();
    if (blank.length > 0) {
      return `Knobs without a value: ${blank.join(', ')} — fill them in or load another template`;
    }
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
    void this.loadTemplates();
  }

  private async loadTemplates(): Promise<void> {
    try {
      this.templates.set(await this.api.getTemplates());
      this.templatesError.set(null);
    } catch (error) {
      // 一覧が読めなくても名前の手入力で続けられる。ただし理由は画面に出す。
      this.templates.set(null);
      this.templatesError.set(
        error instanceof ParamsApiError ? error.message : describeError(error),
      );
    }
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
    this.setDraft(
      params.template,
      flattenKnobs(params.knobs),
      exceptionRows(params.exceptions),
      params.note ?? '',
    );
    this.unsetLeaves.set([]);
    this.unsupported.set([]);
  }

  /** 下書きを丸ごと差し替え、「変更なし」の基準を取り直す。 */
  private setDraft(
    template: string,
    knobs: Map<string, unknown>,
    exceptions: ExceptionDraftRow[],
    note: string,
  ): void {
    this.templateName.set(template);
    this.knobValues.set(knobs);
    this.exceptionsDraft.set(exceptions);
    this.noteText.set(note);
    this.baseline.set(this.snapshot());
  }

  /** `<select>` / 手入力欄の共通ハンドラ。 */
  setNewTemplateInput(event: Event): void {
    this.newTemplateInput.set((event.target as HTMLInputElement | HTMLSelectElement).value);
    this.confirmReload.set(false);
  }

  /**
   * 「Load from template」。下書きに変更があれば、捨ててよいか確認してから(`confirmReload`)。
   * 読み込み前(`draftReady` が偽)や無変更なら、そのまま読む。
   */
  loadFromTemplate(): void {
    if (this.newTemplateInput().trim() === '' || this.defaultsBusy()) return;
    if (this.draftReady() && this.draftDirty() && !this.confirmReload()) {
      this.confirmReload.set(true);
      return;
    }
    void this.fetchDefaults();
  }

  cancelReload(): void {
    this.confirmReload.set(false);
  }

  confirmReloadNow(): void {
    void this.fetchDefaults();
  }

  private async fetchDefaults(): Promise<void> {
    const name = this.newTemplateInput().trim();
    this.defaultsBusy.set(true);
    this.defaultsError.set(null);
    try {
      const defaults = await this.api.getDefaults(name);
      const knobs = flattenKnobs(defaults.knobs);
      // 契約上 unsupported の葉は knobs に居ないはずだが、居ても空欄にする
      // (入れると render が 400 になる値をフォームに残さない)。
      for (const item of defaults.unsupported) knobs.delete(item.knob);
      this.setDraft(defaults.template, knobs, exceptionRows(defaults.exceptions), '');
      this.unsetLeaves.set(defaults.unset);
      this.unsupported.set(defaults.unsupported);
      this.saveResult.set(null);
      this.saveError.set(null);
      this.confirmReload.set(false);
      this.draftReady.set(true);
    } catch (error) {
      // 失敗したら下書きには触らない。サーバの文言をそのまま出す。
      this.defaultsError.set(
        error instanceof ParamsApiError ? error.message : describeError(error),
      );
      this.confirmReload.set(false);
    } finally {
      this.defaultsBusy.set(false);
    }
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

  private clearKnob(key: string): void {
    this.knobValues.update((map) => {
      const next = new Map(map);
      next.delete(key);
      return next;
    });
  }

  /** 空欄の葉に付けるバッジの文言(値があれば `null`)。 */
  blankBadge(key: string): string | null {
    if (this.knobValues().has(key)) return null;
    if (this.unsetLeaves().includes(key)) return 'unset in template';
    if (this.unsupported().some((item) => item.knob === key)) return 'unsupported in template';
    return 'empty';
  }

  /** 空欄(`blankBadge` あり)の enum は、先頭の選択肢が選ばれているように見せない。 */
  isBlank(key: string): boolean {
    return !this.knobValues().has(key);
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
    if (raw.trim() === '') {
      this.clearKnob(field.key); // 空欄は「未設定」のまま(NaN を作らない)。
      return;
    }
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
