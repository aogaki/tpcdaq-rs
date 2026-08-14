/**
 * REST の URL 組み立て(027 の `EndpointsService.apiBase()` は既定 `/api`、
 * `ui-config.json` で絶対 URL に差し替えられる)。
 */
import { apiUrl } from './controller-api';

describe('apiUrl', () => {
  it('既定の same-origin `/api` に繋ぐ', () => {
    expect(apiUrl('/api', 'logbook?since_seq=3')).toBe('/api/logbook?since_seq=3');
    expect(apiUrl('/api', 'logbook/comment')).toBe('/api/logbook/comment');
    expect(apiUrl('/api', 'status')).toBe('/api/status');
  });

  it('`ui-config.json` の絶対 URL でも同じ形になる', () => {
    expect(apiUrl('https://daq-pc.lan/api', 'status')).toBe('https://daq-pc.lan/api/status');
  });

  it('末尾スラッシュがあっても二重にしない', () => {
    expect(apiUrl('/api/', 'status')).toBe('/api/status');
    expect(apiUrl('https://daq-pc.lan/api/', 'status')).toBe('https://daq-pc.lan/api/status');
  });
});
