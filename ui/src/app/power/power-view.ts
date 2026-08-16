/**
 * Power(PSU)— **P6 スコープ**。029 はタブとプレースホルダだけ(発注書 3)。
 *
 * モックの値は出さない(ユーザー決定 2026-08-13)。PSU の実配線が入るまでは、
 * 既に見られるもの(ログブックの `psu` レコード)への導線だけを置く。
 */
import { Component } from '@angular/core';
import { RouterLink } from '@angular/router';

@Component({
  selector: 'app-power-view',
  imports: [RouterLink],
  template: `
    <div class="power">
      <section class="card">
        <h2>Power (PSU)</h2>
        <p class="sub">
          This view belongs to <strong>P6</strong>. HV / LV monitoring and control (the
          <code>psu</code> record of SPEC §9.2 = <code>device</code> / <code>channel</code> /
          <code>event</code> / <code>values{{ '{' }}vmon, imon, vset{{ '}' }}</code
          >) is not wired up yet.
        </p>
        <p class="sub">
          No mock values are shown (user decision 2026-08-13). Once the PSU starts posting to the
          logbook through <code>LogPost</code> (SPEC §2.3), the entries appear in
          <a routerLink="/logbook">Logbook</a> as <code>psu</code>
          — TRIP is highlighted in the attention colour.
        </p>
      </section>
    </div>
  `,
  styles: `
    .power {
      padding: 1rem 1.25rem;
      max-width: 60rem;
    }

    .card {
      border: 1px solid var(--mat-sys-outline-variant);
      border-radius: var(--tpc-radius);
      background: var(--mat-sys-surface-container-low);
      padding: 0.9rem 1rem;
    }

    h2 {
      margin: 0 0 0.5rem;
      font: var(--mat-sys-title-small);
    }

    .sub {
      margin: 0 0 0.5rem;
      color: var(--mat-sys-on-surface-variant);
      font: var(--mat-sys-body-medium);
    }

    a {
      color: var(--tpc-info);
    }
  `,
})
export class PowerView {}
